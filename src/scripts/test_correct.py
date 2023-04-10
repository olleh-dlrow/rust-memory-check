"""
python src/scripts/test_correct.py
"""

import os
import sys
import tqdm
if os.system("cargo build"):
    exit(-1)

dir_path = './examples/correctness'

sample_paths = [os.path.join(dir_path, fname) for fname in os.listdir(dir_path) if fname.endswith('.rs')]

# 遍历sample_paths中的每个文件，加进度条
for sample_path in tqdm.tqdm(sample_paths):
    cmd = "cargo run --bin mc " + sample_path
    if os.system(cmd):
        print("Failed to compile " + sample_path)
        exit(-1)
    else:
        print("Successfully compiled " + sample_path)
