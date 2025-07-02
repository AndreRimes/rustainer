use clap::{Arg, ArgMatches, Command};
use std::process;

mod actions;

#[tokio::main]
async fn main() {
    let matches = Command::new("rustainer")
        .version("0.1.0")
        .author("Your Name <your.email@example.com>")
        .about("A container runtime written in Rust")
        .subcommand(
            Command::new("run")
                .about("Run a container from an image")
                .arg(
                    Arg::new("image")
                        .help("Container image to run")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("name")
                        .short('n')
                        .long("name")
                        .help("Container name")
                        .value_name("NAME"),
                )
                .arg(
                    Arg::new("detach")
                        .short('d')
                        .long("detach")
                        .help("Run container in background")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("interactive")
                        .short('i')
                        .long("interactive")
                        .help("Keep STDIN open even if not attached")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("tty")
                        .short('t')
                        .long("tty")
                        .help("Allocate a pseudo-TTY")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("env")
                        .short('e')
                        .long("env")
                        .help("Set environment variables")
                        .value_name("KEY=VALUE")
                        .action(clap::ArgAction::Append),
                )
                .arg(
                    Arg::new("volume")
                        .short('v')
                        .long("volume")
                        .help("Bind mount a volume")
                        .value_name("HOST:CONTAINER")
                        .action(clap::ArgAction::Append),
                )
                .arg(
                    Arg::new("port")
                        .short('p')
                        .long("port")
                        .help("Publish a container's port(s) to the host")
                        .value_name("HOST:CONTAINER")
                        .action(clap::ArgAction::Append),
                )
                .arg(
                    Arg::new("command")
                        .help("Command to run in the container")
                        .index(2)
                        .action(clap::ArgAction::Append),
                ),
        )
        .subcommand(
            Command::new("pull")
                .about("Pull an image from a registry")
                .arg(
                    Arg::new("image")
                        .help("Image to pull (e.g., nginx:latest)")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(Command::new("images").about("List locally stored images"))
        .get_matches();

    match matches.subcommand() {
        Some(("run", sub_matches)) => {
            if let Err(e) = handle_run_command(sub_matches).await {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        Some(("pull", sub_matches)) => {
            if let Err(e) = handle_pull_command(sub_matches).await {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        Some(("images", _)) => {
            if let Err(e) = handle_images_command().await {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "No subcommand provided. Use 'rustainer pull <image>' or 'rustainer run <image>'."
            );
            process::exit(1);
        }
    }
}

async fn handle_run_command(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    let image = matches.get_one::<String>("image").unwrap();

    println!("🚀 Starting container runtime...");
    println!("📦 Image: {}", image);

    Ok(())
}

async fn handle_pull_command(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    let image = matches.get_one::<String>("image").unwrap();
    actions::pull::pull_image(image).await?;
    Ok(())
}

async fn handle_images_command() -> Result<(), Box<dyn std::error::Error>> {
    actions::images::list_images().await?;
    Ok(())
}
