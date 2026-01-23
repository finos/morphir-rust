//! Help and banner display utilities for the Morphir CLI.

use owo_colors::{OwoColorize, XtermColors};

/// Print the Morphir ASCII art banner with branded colors.
pub fn print_banner() {
    // Morphir brand colors: blue (#00A3E0) and orange (#F26522)
    let blue = XtermColors::from(33); // Bright blue (xterm 33)
    let orange = XtermColors::from(208); // Orange (xterm 208)

    // ASCII art "morphir" with "morph" in blue and "ir" in orange
    println!();
    println!(
        "  {}{}",
        "_ __ ___   ___  _ __ _ __ | |__".color(blue),
        "(_)_ __".color(orange)
    );
    println!(
        " {}{}",
        "| '_ ` _ \\ / _ \\| '__| '_ \\| '_ \\".color(blue),
        "| | '__|".color(orange)
    );
    println!(
        " {}{}",
        "| | | | | | (_) | |  | |_) | | | ".color(blue),
        "| | |".color(orange)
    );
    println!(
        " {}{}",
        "|_| |_| |_|\\___/|_|  | .__/|_| |_".color(blue),
        "|_|_|".color(orange)
    );
    println!("                     {}", "|_|".color(blue));
    println!(
        "  v{} (built {})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE")
    );
    println!();
}

/// Print full help including experimental commands.
pub fn print_full_help<C: clap::CommandFactory>() {
    let mut cmd = C::command();
    // Unhide the experimental commands
    for subcommand in cmd.get_subcommands_mut() {
        if subcommand.get_name() == "validate"
            || subcommand.get_name() == "generate"
            || subcommand.get_name() == "transform"
        {
            *subcommand = subcommand.clone().hide(false);
        }
    }
    println!("Note: Commands marked [Experimental] are not yet fully implemented.\n");
    cmd.print_help().ok();
}

/// Determine if the banner should be shown based on command-line arguments.
pub fn should_show_banner(args: &[String]) -> bool {
    args.len() == 1
        || args.iter().any(|a| a == "--help" || a == "-h")
        || args.iter().any(|a| a == "--help-all")
        || args.iter().any(|a| a == "--version" || a == "-V")
        || (args.len() == 2 && args.iter().any(|a| a == "help"))
        || should_show_full_help(args)
}

/// Determine if full help (including experimental commands) should be shown.
pub fn should_show_full_help(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help-all")
        || (args.iter().any(|a| a == "help")
            && args
                .iter()
                .any(|a| a == "--all" || a == "--full" || a == "--experimental"))
}
