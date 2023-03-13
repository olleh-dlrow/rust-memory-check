import os
import sys
if os.system("cargo build"):
    exit(-1)

file_path = sys.argv[1]
os.system(f"cargo run --bin mc {file_path}")
