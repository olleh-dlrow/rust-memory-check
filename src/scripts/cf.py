import os
import sys
if os.system("cargo build"):
    exit(-1)

args = sys.argv[1:]
cmd = "cargo run --bin mc " + " ".join(args)
os.system(cmd)
