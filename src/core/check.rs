use colored::Colorize;
use rustc_hir::def_id::DefId;
use rustc_span::Span;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use super::cfg::ControlFlowGraph;
use super::{CtxtSenSpanInfo, DropObjectId, GlobalBasicBlockId, GlobalProjectionId};
use crate::core::analysis::AnalysisContext;
use crate::core::utils;
use itertools::Itertools;

pub fn check_memory_bug(ctxt: &AnalysisContext) -> CheckInfo {
    let mut check_info = CheckInfo::new();

    check_info.uaf_infos = check_uaf(&ctxt);
    check_info.df_infos = check_df(&ctxt);

    check_info
}

pub fn merge_check_info(
    cfgs: &HashMap<DefId, ControlFlowGraph>,
    check_infos: &HashMap<DefId, CheckInfo>,
) -> CheckResult {
    let get_var_name = |proj_id: GlobalProjectionId| {
        let cfg = cfgs.get(&proj_id.g_local_id.def_id).unwrap();
        let local_info = cfg.local_infos.get(&proj_id.g_local_id.local_id).unwrap();
        local_info.var_name.clone()
    };

    // handle uaf info
    let mut uaf_results = HashMap::<UafSpan, HashSet<UafResult>>::new();
    let uaf_into_iter = check_infos
        .iter()
        .map(|(_, check_info)| check_info.uaf_infos.iter())
        .flatten();
    for uaf_info in uaf_into_iter {
        let uaf_span = UafSpan::new(uaf_info.deref_span.span, uaf_info.drop_span.span);
        let uaf_result = UafResult::new(
            uaf_info.deref_span.span,
            get_var_name(uaf_info.deref_proj_id),
            uaf_info.drop_span.span,
            get_var_name(uaf_info.drop_obj_id.into()),
        );
        if !uaf_results.contains_key(&uaf_span) {
            uaf_results.insert(uaf_span, Some(uaf_result).into_iter().collect());
        } else {
            let uaf_results_with_span = uaf_results.get_mut(&uaf_span).unwrap();
            if uaf_result.has_var_name() {
                // the first does not have var name, pop it
                if uaf_results_with_span.len() == 1 && !uaf_results_with_span.iter().next().unwrap().has_var_name() {
                    uaf_results_with_span.clear();
                }
                uaf_results_with_span.insert(uaf_result);
            }
        }
    }

    // handle df info
    let mut df_results = HashMap::<DfSpan, HashSet<DfResult>>::new();
    let df_into_iter = check_infos
        .iter()
        .map(|(_, check_info)| check_info.df_infos.iter())
        .flatten();
    for df_info in df_into_iter {
        let df_span = DfSpan::new(df_info.first_drop_span.span, df_info.then_drop_span.span);
        let df_result = DfResult::new(
            df_info.first_drop_span.span,
            get_var_name(df_info.first_drop_obj_id.into()),
            df_info.then_drop_span.span,
            get_var_name(df_info.then_drop_obj_id.into()),
        );
        if !df_results.contains_key(&df_span) {
            df_results.insert(df_span, Some(df_result).into_iter().collect());
        } else {
            let df_results_with_span = df_results.get_mut(&df_span).unwrap();
            if df_result.has_var_name() {
                // the first does not have var name, pop it
                if df_results_with_span.len() == 1 && !df_results_with_span.iter().next().unwrap().has_var_name() {
                    df_results_with_span.clear();
                }
                df_results_with_span.insert(df_result);
            }
        }        
    }


    let mut check_result = CheckResult::new();

    check_result.uaf_results = uaf_results;     
    check_result.df_results = df_results;     

    check_result
}

pub fn output_check_result(check_result: &CheckResult) {
    // handle uaf
    let uaf_result_iter = check_result.uaf_results.iter().map(|(_, result)| result.iter()).flatten();
    for uaf_result in uaf_result_iter {
        output_level_text("warning", "use after free memory bug may exists");
        let (filename, line_range, column_range) = utils::parse_span(&uaf_result.drop_span);
        let problem_text = match &uaf_result.drop_var_name {
            Some(var_name) => format!("first drop here, relative variable: {}", var_name),
            None => "first drop here.".to_string(),
        };
        output_code_and_problem_info(&filename, line_range, column_range, &problem_text);

        let (filename, line_range, column_range) = utils::parse_span(&uaf_result.deref_span);
        let problem_text = match &uaf_result.deref_var_name {
            Some(var_name) => format!("then dereference here, relative variable: {}", var_name),
            None => "then dereference here.".to_string(),
        };
        output_code_and_problem_info(&filename, line_range, column_range, &problem_text);
    }

    // handle df
    let df_result_iter = check_result.df_results.iter().map(|(_, result)| result.iter()).flatten();
    for df_result in df_result_iter {
        output_level_text("warning", "double free memory bug may exists");
        let (filename, line_range, column_range) = utils::parse_span(&df_result.first_drop_span);
        let problem_text = match &df_result.first_drop_var_name {
            Some(var_name) => format!("first drop here, relative variable: {}", var_name),
            None => "first drop here.".to_string(),
        };
        output_code_and_problem_info(&filename, line_range, column_range, &problem_text);

        let (filename, line_range, column_range) = utils::parse_span(&df_result.then_drop_span);
        let problem_text = match &df_result.then_drop_var_name {
            Some(var_name) => format!("then drop here, relative variable: {}", var_name),
            None => "then drop here.".to_string(),
        };
        output_code_and_problem_info(&filename, line_range, column_range, &problem_text);
    }

}


#[derive(Debug)]
pub struct CheckInfo {
    pub uaf_infos: Vec<UafInfo>,
    pub df_infos: Vec<DfInfo>,
}

impl CheckInfo {
    pub fn new() -> Self {
        Self {
            uaf_infos: Vec::new(),
            df_infos: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct CheckResult {
    pub uaf_results: HashMap<UafSpan, HashSet<UafResult>>,
    pub df_results: HashMap<DfSpan, HashSet<DfResult>>,
}

impl CheckResult {
    pub fn new() -> Self {
        Self {
            uaf_results: HashMap::new(),
            df_results: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UafResult {
    pub deref_span: Span,
    pub deref_var_name: Option<String>,
    pub drop_span: Span,
    pub drop_var_name: Option<String>,
}

impl UafResult {
    pub fn new(
        deref_span: Span,
        deref_var_name: Option<String>,
        drop_span: Span,
        drop_var_name: Option<String>,
    ) -> Self {
        Self {
            deref_span,
            deref_var_name,
            drop_span,
            drop_var_name,
        }
    }

    pub fn has_var_name(&self) -> bool {
        self.deref_var_name.is_some() || self.drop_var_name.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub struct UafSpan {
    pub deref_span: Span,
    pub drop_span: Span,
}

impl UafSpan {
    pub fn new(deref_span: Span, drop_span: Span) -> Self {
        Self {
            deref_span,
            drop_span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DfResult {
    pub first_drop_span: Span,
    pub first_drop_var_name: Option<String>,
    pub then_drop_span: Span,
    pub then_drop_var_name: Option<String>,
}

impl DfResult {
    pub fn new(
        first_drop_span: Span,
        first_drop_var_name: Option<String>,
        then_drop_span: Span,
        then_drop_var_name: Option<String>,
    ) -> Self {
        Self {
            first_drop_span,
            first_drop_var_name,
            then_drop_span,
            then_drop_var_name,
        }
    }

    pub fn has_var_name(&self) -> bool {
        self.first_drop_var_name.is_some() || self.then_drop_var_name.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub struct DfSpan {
    pub first_drop_span: Span,
    pub then_drop_span: Span,
}

impl DfSpan {
    pub fn new(first_drop_span: Span, then_drop_span: Span) -> Self {
        Self {
            first_drop_span,
            then_drop_span,
        }
    }
}

trait SameSpan {
    fn is_same_span(&self, other: &Self) -> bool;
}

#[derive(Debug)]
pub struct UafInfo {
    pub deref_proj_id: GlobalProjectionId,
    pub deref_span: CtxtSenSpanInfo,
    pub drop_obj_id: DropObjectId,
    pub drop_span: CtxtSenSpanInfo,
}

impl UafInfo {
    pub fn new(
        deref_proj_id: GlobalProjectionId,
        deref_span: CtxtSenSpanInfo,
        drop_obj_id: DropObjectId,
        drop_span: CtxtSenSpanInfo,
    ) -> Self {
        Self {
            deref_proj_id,
            deref_span,
            drop_obj_id,
            drop_span,
        }
    }
}

impl SameSpan for UafInfo {
    fn is_same_span(&self, other: &Self) -> bool {
        self.deref_span.span == other.deref_span.span && self.drop_span.span == other.drop_span.span
    }
}

#[derive(Debug)]
pub struct DfInfo {
    pub first_drop_obj_id: DropObjectId,
    pub first_drop_span: CtxtSenSpanInfo,

    pub then_drop_obj_id: DropObjectId,
    pub then_drop_span: CtxtSenSpanInfo,
}

impl DfInfo {
    pub fn new(
        first_drop_obj_id: DropObjectId,
        first_drop_span: CtxtSenSpanInfo,
        then_drop_obj_id: DropObjectId,
        then_drop_span: CtxtSenSpanInfo,
    ) -> Self {
        Self {
            first_drop_obj_id,
            first_drop_span,
            then_drop_obj_id,
            then_drop_span,
        }
    }
}

impl SameSpan for DfInfo {
    fn is_same_span(&self, other: &Self) -> bool {
        self.first_drop_span.span == other.first_drop_span.span
            && self.then_drop_span.span == other.then_drop_span.span
    }
}

fn check_df(ctxt: &AnalysisContext) -> Vec<DfInfo> {
    let mut df_infos = Vec::new();

    for first_drop_obj_id in ctxt.pfg.multi_drop_objects.iter() {
        let points_to = &ctxt
            .pfg
            .get_projection_node((*first_drop_obj_id).into())
            .points_to;
        // If the object is not pointed by any other object, it is not a double free.
        if points_to.len() <= 1 {
            continue;
        }

        // exclude itself
        let then_drop_objs = points_to
            .iter()
            .filter(|&obj_id| obj_id != first_drop_obj_id)
            .map(|obj_id| *obj_id)
            .collect::<HashSet<DropObjectId>>();

        let first_drop_span_infos = &ctxt
            .pfg
            .get_projection_node((*first_drop_obj_id).into())
            .cs_drop_spans;

        for then_drop_obj in then_drop_objs.iter() {
            let then_drop_span_infos = &ctxt
                .pfg
                .get_projection_node((*then_drop_obj).into())
                .cs_drop_spans;

            let product = first_drop_span_infos
                .iter()
                .cartesian_product(then_drop_span_infos.iter());

            for (first_drop_span_info, then_drop_span_info) in product {
                let first_drop_bb_id = GlobalBasicBlockId::new(
                    first_drop_span_info.def_id,
                    first_drop_span_info.basic_block_id,
                );
                let then_drop_bb_id = GlobalBasicBlockId::new(
                    then_drop_span_info.def_id,
                    then_drop_span_info.basic_block_id,
                );

                // if first drop object can arrive then drop object, it is a double free.
                if utils::can_basic_block_arrive(
                    &ctxt.cfgs,
                    &mut HashSet::new(),
                    first_drop_bb_id,
                    then_drop_bb_id,
                ) {
                    let target_info = DfInfo::new(
                        *first_drop_obj_id,
                        first_drop_span_info.clone(),
                        *then_drop_obj,
                        then_drop_span_info.clone(),
                    );

                    // if !contains_same_span(&df_infos, &target_info) {
                    df_infos.push(target_info);
                    // }
                }

                // if then drop object can arrive first drop object, it is a double free.
                if utils::can_basic_block_arrive(
                    &ctxt.cfgs,
                    &mut HashSet::new(),
                    then_drop_bb_id,
                    first_drop_bb_id,
                ) {
                    let target_info = DfInfo::new(
                        *then_drop_obj,
                        then_drop_span_info.clone(),
                        *first_drop_obj_id,
                        first_drop_span_info.clone(),
                    );

                    // if !contains_same_span(&df_infos, &target_info) {
                    df_infos.push(target_info);
                    // }
                }
            }
        }
    }

    df_infos
}

fn check_uaf(ctxt: &AnalysisContext) -> Vec<UafInfo> {
    let mut uaf_infos = Vec::new();

    let mut add_uaf_infos = |deref_proj_id: GlobalProjectionId,
                             deref_span_info: &CtxtSenSpanInfo| {
        for drop_obj_id in ctxt.pfg.get_projection_node(deref_proj_id).points_to.iter() {
            let drop_span_infos = &ctxt
                .pfg
                .get_projection_node((*drop_obj_id).into())
                .cs_drop_spans;

            for drop_span_info in drop_span_infos.iter() {
                let drop_bb_id =
                    GlobalBasicBlockId::new(drop_span_info.def_id, drop_span_info.basic_block_id);
                let deref_bb_id = GlobalBasicBlockId::new(
                    deref_span_info.def_id,
                    deref_span_info.basic_block_id,
                );
                if utils::can_basic_block_arrive(
                    &ctxt.cfgs,
                    &mut HashSet::new(),
                    drop_bb_id,
                    deref_bb_id,
                ) && drop_bb_id != deref_bb_id {
                    let target_info = UafInfo::new(
                        deref_proj_id,
                        deref_span_info.clone(),
                        *drop_obj_id,
                        drop_span_info.clone(),
                    );

                    // if !contains_same_span(&uaf_infos, &target_info) {
                    uaf_infos.push(target_info);
                    // }
                }
            }
        }
    };

    for deref_edge_info in ctxt.pfg.deref_edges.iter() {
        let from = deref_edge_info.from;
        let to = deref_edge_info.to;

        let neighbor_info = ctxt
            .pfg
            .get_projection_node(from)
            .neighbors
            .get(&to)
            .unwrap();

        if deref_edge_info.is_deref.0 {
            add_uaf_infos(from, &neighbor_info.span_info);
        }

        if deref_edge_info.is_deref.1 {
            add_uaf_infos(to, &neighbor_info.span_info);
        }
    }

    uaf_infos
}

fn contains_same_span<T: SameSpan>(infos: &Vec<T>, target: &T) -> bool {
    for info in infos.iter() {
        if info.is_same_span(target) {
            return true;
        }
    }

    false
}

pub fn output_uaf_infos(uaf_infos: &Vec<UafInfo>) {
    log::debug!("uaf_infos: {:#?}", uaf_infos);
    for uaf_info in uaf_infos.iter() {
        let (deref_file_path, deref_line_range, deref_col_range) =
            utils::parse_span(&uaf_info.deref_span.span);
        let (drop_file_path, drop_line_range, drop_col_range) =
            utils::parse_span(&uaf_info.drop_span.span);

        let deref_line_char_width = std::cmp::max(
            deref_line_range.0.to_string().len(),
            deref_line_range.1.to_string().len(),
        );

        let drop_line_char_width = std::cmp::max(
            drop_line_range.0.to_string().len(),
            drop_line_range.1.to_string().len(),
        );

        println!(
            "{}({}) {}",
            "warning:".yellow().bold(),
            "memory check".cyan().bold(),
            "use after free memory bug may exists".bold()
        );
        // drop output
        println!(
            "{}{} {}:{}:{}",
            " ".repeat(drop_line_char_width),
            "--> ".blue(),
            drop_file_path,
            drop_line_range.0,
            drop_col_range.0
        );
        println!("{} {}", " ".repeat(drop_line_char_width), "|".blue());
        for (i, line) in utils::get_lines_in_file(&drop_file_path, drop_line_range)
            .iter()
            .enumerate()
        {
            let i = i + drop_line_range.0;
            println!("{} {}{}", i.to_string().blue(), "|".blue(), line);
        }
        println!(
            "{} {}{}{} {}",
            " ".repeat(drop_line_char_width),
            "|".blue(),
            " ".repeat(drop_col_range.0 - 1),
            "^".repeat(drop_col_range.1 - drop_col_range.0).yellow(),
            " first drop here".yellow()
        );
        println!("{} {}", " ".repeat(drop_line_char_width), "|".blue());
        println!("");

        // deref output
        println!(
            "{}{} {}:{}:{}",
            " ".repeat(deref_line_char_width),
            "--> ".blue(),
            deref_file_path,
            deref_line_range.0,
            deref_col_range.0
        );
        println!("{} {}", " ".repeat(deref_line_char_width), "|".blue());
        for (i, line) in utils::get_lines_in_file(&deref_file_path, deref_line_range)
            .iter()
            .enumerate()
        {
            let i = i + deref_line_range.0;
            println!("{} {}{}", i.to_string().blue(), "|".blue(), line);
        }
        println!(
            "{} {}{}{} {}",
            " ".repeat(deref_line_char_width),
            "|".blue(),
            " ".repeat(deref_col_range.0 - 1),
            "^".repeat(deref_col_range.1 - deref_col_range.0).yellow(),
            " then dereference here".yellow()
        );
        println!("{} {}", " ".repeat(deref_line_char_width), "|".blue());
        println!("");
    }
}

pub fn output_df_infos(df_infos: &Vec<DfInfo>) {
    log::debug!("df infos: {:#?}", df_infos);
    for df_info in df_infos.iter() {
        let (then_drop_file_path, then_drop_line_range, then_drop_col_range) =
            utils::parse_span(&df_info.then_drop_span.span);
        let (first_drop_file_path, first_drop_line_range, first_drop_col_range) =
            utils::parse_span(&df_info.first_drop_span.span);

        let then_drop_line_char_width = std::cmp::max(
            then_drop_line_range.0.to_string().len(),
            then_drop_line_range.1.to_string().len(),
        );

        let first_drop_line_char_width = std::cmp::max(
            first_drop_line_range.0.to_string().len(),
            first_drop_line_range.1.to_string().len(),
        );

        println!(
            "{}({}) {}",
            "warning:".yellow().bold(),
            "memory check".cyan().bold(),
            "double free memory bug may exists".bold()
        );
        // first drop output
        println!(
            "{}{} {}:{}:{}",
            " ".repeat(first_drop_line_char_width),
            "--> ".blue(),
            first_drop_file_path,
            first_drop_line_range.0,
            first_drop_col_range.0
        );
        println!("{} {}", " ".repeat(first_drop_line_char_width), "|".blue());
        for (i, line) in utils::get_lines_in_file(&first_drop_file_path, first_drop_line_range)
            .iter()
            .enumerate()
        {
            let i = i + first_drop_line_range.0;
            println!("{} {}{}", i.to_string().blue(), "|".blue(), line);
        }
        println!(
            "{} {}{}{} {}",
            " ".repeat(first_drop_line_char_width),
            "|".blue(),
            " ".repeat(first_drop_col_range.0 - 1),
            "^".repeat(first_drop_col_range.1 - first_drop_col_range.0)
                .yellow(),
            " first drop here".yellow()
        );
        println!("{} {}", " ".repeat(first_drop_line_char_width), "|".blue());
        println!("");

        // then drop output
        println!(
            "{}{} {}:{}:{}",
            " ".repeat(then_drop_line_char_width),
            "--> ".blue(),
            then_drop_file_path,
            then_drop_line_range.0,
            then_drop_col_range.0
        );
        println!("{} {}", " ".repeat(then_drop_line_char_width), "|".blue());
        for (i, line) in utils::get_lines_in_file(&then_drop_file_path, then_drop_line_range)
            .iter()
            .enumerate()
        {
            let i = i + then_drop_line_range.0;
            println!("{} {}{}", i.to_string().blue(), "|".blue(), line);
        }
        println!(
            "{} {}{}{} {}",
            " ".repeat(then_drop_line_char_width),
            "|".blue(),
            " ".repeat(then_drop_col_range.0 - 1),
            "^".repeat(then_drop_col_range.1 - then_drop_col_range.0)
                .yellow(),
            " then drop here".yellow()
        );
        println!("{} {}", " ".repeat(then_drop_line_char_width), "|".blue());
        println!("");
    }
}

// level print
// \yellow$level:   \cyan(memroy\scheck)   \normal\s$text
pub fn output_level_text(level: &str, text: &str) {
    let s = format!("{}:", level);
    utils::print_with_color(&s, Color::Yellow).unwrap();
    
    let s = "(memory check)";
    utils::print_with_color(&s, Color::Cyan).unwrap();
    
    let s = format!(" {}", text);
    utils::println_with_color(&s, Color::White).unwrap();
}

/*
// code and problem
\blue(\s).repeat($max_line_char_width)-->\s   \normal$filename:$line_lo:$col_lo

\blue(\s).repeat($max_line_char_width+1)|\s

\blue$line_nb\s|\s   \normal$line_text

\blue(\s).repeat($max_line_char_width+1)|\s   \yellow(\s).repeat($col_start-1)(^).repeat($col_end-$col_start+1)\s$problem_text(if line_nb==line_hi)

\blue(\s).repeat($max_line_char_width+1)|\s

 */
fn output_code_and_problem_info(
    filename: &str,
    line_range: (usize, usize),
    col_range: (usize, usize),
    problem_text: &str,
) {
    let max_line_char_width = std::cmp::max(line_range.0.to_string().len(), line_range.1.to_string().len());

    let lines = utils::get_lines_in_file(filename, line_range);

    // code and problem
    // print -->
    let s = format!("{}--> ", " ".repeat(max_line_char_width));
    utils::print_with_color(&s, Color::Blue).unwrap();
    let s = format!("{}:{}:{}", filename, line_range.0, col_range.0);
    utils::println_with_color(&s, Color::White).unwrap();

    // print |
    let s = format!("{}| ", " ".repeat(max_line_char_width + 1));
    utils::println_with_color(&s, Color::Blue).unwrap();

    // print lines
    for (i, line) in lines.iter().enumerate() {
        let i = i + line_range.0;

        // print line
        let s = format!("{} | ", i);
        utils::print_with_color(&s, Color::Blue).unwrap();
        let s = format!("{}", line);
        utils::println_with_color(&s, Color::White).unwrap();

        // print ^
        let col_start = if i == line_range.0 { col_range.0 } else { 1 };
        let col_end = if i == line_range.1 { col_range.1 } else { line.len() };
        let s = format!("{}| ", " ".repeat(max_line_char_width + 1));
        utils::print_with_color(&s, Color::Blue).unwrap();
        let s = format!("{}{} ", " ".repeat(col_start - 1), "^".repeat(col_end - col_start + 1));
        utils::print_with_color(&s, Color::Yellow).unwrap();
        if i == line_range.1 {
            let s = format!("{}", problem_text);
            utils::print_with_color(&s, Color::Yellow).unwrap();
        } 
        utils::println_with_color("", Color::White).unwrap();

        // print |
        let s = format!("{}| ", " ".repeat(max_line_char_width + 1));
        utils::println_with_color(&s, Color::Blue).unwrap();
    }
}

