"""
python src/scripts/cc.py --manifest-path ./examples/correctness/crates/test01/Cargo.toml
"""


import os
import sys
from termcolor import colored

def print_err(s: str):
    print(colored(s, 'red'))

def print_ok(s: str):
    print(colored(s, 'green'))
    

print("build cargo-mc and mc...")
if os.system("cargo build"):
    print_err("build failed.")
    exit(-1)
print_ok(f"build success.")



args = sys.argv[1:]
idx = args.index("--manifest-path")
target_crate = args[idx + 1]

print(f"check target {target_crate}...")
os.system(f"cargo clean --manifest-path {target_crate}")
if os.system(f"cargo run --bin cargo-mc mc " + " ".join(args)):
    print_err(f"check target {target_crate} failed.")
    exit(-1)

print_ok(f"check target {target_crate} success.")
