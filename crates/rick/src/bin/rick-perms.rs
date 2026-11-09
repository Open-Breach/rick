//! `rick-perms`, the privileged helper: the one program Rick runs as the
//! administrator, through the platform's prompt, when a permission change
//! needs it. It reads a request file naming entries and the changes to
//! make, records what each entry had, applies the changes with the same
//! platform code Rick uses, and prints the reply
//! (`platform::perms::helper`). It validates everything it reads and does
//! nothing else, so what runs elevated stays small.

#![forbid(unsafe_code)]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(platform::perms::helper::main(
        &platform::Native::new(),
        &args,
    ));
}
