use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "scythe", version, about = "SQL-to-code generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate code from SQL schema and queries
    Generate {
        /// Path to config file
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
    },
    /// Migrate from sqlc to scythe format
    Migrate {
        /// Path to sqlc config file
        #[arg(default_value = "sqlc.yaml")]
        sqlc_config: String,
    },
    /// Validate SQL without generating code
    Check {
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Generate { config } => scythe::commands::generate::run_generate(&config),
        Commands::Migrate { sqlc_config } => {
            scythe::commands::migrate::run_migrate(std::path::Path::new(&sqlc_config))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        }
        Commands::Check { config } => scythe::commands::generate::run_check(&config),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
