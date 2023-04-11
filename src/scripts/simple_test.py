import os
from termcolor import colored

def print_err(s: str):
    print(colored(s, 'red'))

def print_ok(s: str):
    print(colored(s, 'green'))

print("install cargo-mc and mc...")
if os.system("cargo install --path ."):
    print_err("install failed.")
    exit(-1)

sample_file = "examples/correctness/sample01.rs"
print(f"check sample {sample_file}...")
if os.system(f"cargo run --bin mc {sample_file}"):
    print_err(f"check sample {sample_file} failed.")
    exit(-1)

target_crate = "~/dev/rust_project/rand/Cargo.toml"
# target_crate = "~/dev/static_analysis/rust/examples/use_after_free/RUSTSEC-2021-0130/Cargo.toml"
print(f"check target {target_crate}...")
os.system(f"cargo clean --manifest-path {target_crate}")
if os.system(f"cargo mc --manifest-path {target_crate}"):
    print_err(f"check target {target_crate} failed.")
    exit(-1)

print_ok("all tests passed.")
