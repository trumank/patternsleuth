use std::process::Command;

fn main() {
    if let Some(git_hash) = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
    {
        println!("cargo:rustc-env=GIT_HASH={}", git_hash);
    }

    if Command::new("git")
        .args(["diff-index", "--quiet", "HEAD"])
        .status()
        .ok()
        .is_some_and(|status| !status.success())
    {
        println!("cargo:rustc-env=GIT_DIRTY=true");
    }
}
