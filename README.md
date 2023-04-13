# rust-memory-check

## 特性

- 程序静态分析
- use after free、dangling pointer检测定位
- double free检测定位
- ...



## 使用

### linux

#### 安装


```bash
cargo install -- path .
```



#### 项目分析

```bash
cargo mc --manifest-path CARGO_TOML_PATH --entries [ENTRY_FUNCTION_NAME, ...]
```



#### Debug

在rust-memory-check文件夹下：

分析单个文件：

```bash
cargo run --bin mc FILEPATH --entries [ENTRY_FUNCTION_NAME, ...]
```

分析项目：

```bash
cargo run --bin cargo-mc mc --manifest-path CARGO_TOML_PATH --entries [ENTRY_FUNCTION_NAME, ...]
```

其他选项：

...



### windows

暂未测试



## 算法

基于MIR的控制流图和函数调用图的构建

基于别名分析和过程间分析的指针图构建

基于流非敏感，上下文敏感和域敏感的数据结构分析算法的改进



别名分析+过程间分析的简要过程：

```
AliasAnalysis()
	WL = []										
	PFG = {}
	S: set of reachable statements = {}
	RM: set of reachable methods = {}
	CG: call graph edges = {}
	m^entry: entry call

	ProcessCalls([]: m^entry)
	
	while WL is not empty do
		remove <c: n, pts> from WL
		delta = pts - pt(c: n)
		Propagate(c: n, delta)
```



函数调用处理：

```
ProcessCalls(c: m^entry)
	WL = [c: m^entry]
	RM: context sensitive reachable method = {}
	while WL is not empty do 
		remove m from WL
		if c: m not in RM then
			AddReachable(c: m)
			

           foreach l: r = cs(a1, ..., an) in c: m do
				c^t = Select(c, l:=callsite, c':oi)
                if cs in extern crate then		
                    foreach parameter ai of cs do
                        if op is move or ref or addressof then 
                            AddEdge(c: ai, c: r, span)
						elseif op is copy and project(y) is ptr or ref then
							AddEdge(c: ai, c: r, span)

                elseif cs in local crate then		
                	add c^t: cs to WL
                	foreach parameter pi of cs do
                		if op is move or ref or addressof then
                			AddEdge(c: ai, c^t: pi, span)
						elseif op is copy and project(y) is ptr or ref then
							AddEdge(c: ai, c^t: pi, span)
                		if has ret then
                			AddEdge(c^t: cs_ret, c: r, span)
```



指针集传播：

```
Propagate(c: n, pts)
	if pts is not empty then
		pt(c: n) union = pts
		if c: n not in PFG.multi_drop_objs and pt(c: n) has other drop_obj(pt(c: n).len > 1) then
			add c: n to PFG.multi_drop_objs
		// diffuse at sub level
		foreach c': proj in c: n.projections do
			if c: n is the prefix of c': proj and c == c' and c: n != c': proj then	// not include itself
				add <c': proj, pts> to WL

		// diffuse at same level
		foreach c': proj in c: n.projections do
			if c': proj in the prefix of c: n and c == c' then	// include itself
				diff_projections = c: n.proj.value - c': proj.value
				foreach c': proj -> c'': s in PFG do
                	c'': t = add or update c'': s + diff_projections to PFG
                	add <c'': t, pts> to WL			
```





## 示例

### UAF

样本：

```rust
use std::ptr;

struct MyStruct {
    x: i32,
    y: Box<i32>
}

impl MyStruct {
    fn new(x: i32) -> Self {
        Self {
            x,
            y: Box::new(x)
        }
    }

    fn as_ptr(&self) -> *const i32 {
        &self.x as *const i32
    }

    fn get_y(&self) -> *const i32 {
        unsafe { self.y.as_ref() as *const i32 }
    }
}

fn main() {
    let x = 12;
    let p = match x {
        12 => MyStruct::new(444).get_y(),
        _  => ptr::null()
    };
    let y = 1;
    let arr = [1, 2, 3, 4, 5, 7, 8, 9, 0];
    unsafe {
        println!("{}", *p);
    }
}
```

输出结果：

```bash
info:(memory check) analysis from entries:
 - sample01::main
warning:(memory check) use after free memory bug may exists
  --> examples/use_after_free/sample01.rs:28:40
   | 
28 |         12 => MyStruct::new(444).get_y(),
   |                                        ^^ first drop here.
   | 
  --> examples/use_after_free/sample01.rs:34:24
   | 
34 |         println!("{}", *p);
   |                        ^^^ then dereference here, relative variable: p
   | 
info:(memory check) total: 1 uaf bugs, 0 df bugs
```



### DF

样本：

```rust
use std::vec::Vec;
use std::ptr;
struct FILE {
    buf: Vec<u8>
}

fn main() {
    let mut a = vec![1,2];
    let ptr = a.as_mut_ptr();
    unsafe{
        let _v = Vec::from_raw_parts(ptr, 2, 2);
    }
}

fn foo() {
    let f1 = FILE{buf: vec![0u8; 100]};
    let f2 = unsafe {ptr::read::<FILE>(&f1)};
}

```

分析结果：

```bash
info:(memory check) auto detect entries
info:(memory check) analysis from entries:
 - sample04::main
 - sample04::foo
warning:(memory check) double free memory bug may exists
  --> examples/double_free/sample04.rs:18:1
   | 
18 | }
   | ^^ first drop here, relative variable: f2
   | 
  --> examples/double_free/sample04.rs:18:1
   | 
18 | }
   | ^^ then drop here, relative variable: f1
   | 
warning:(memory check) double free memory bug may exists
  --> examples/double_free/sample04.rs:12:5
   | 
12 |     }
   |     ^^ first drop here, relative variable: _v
   | 
  --> examples/double_free/sample04.rs:13:1
   | 
13 | }
   | ^^ then drop here, relative variable: a
   | 
info:(memory check) total: 0 uaf bugs, 2 df bugs
```



## 参考

...

