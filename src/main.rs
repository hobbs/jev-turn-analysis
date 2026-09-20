use clap::Parser;
#[tokio::main]
async fn main() {
    let cli = jta::cli::Cli::parse();
    match jta::cli::execute(cli).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            let msg = e.to_string();
            std::process::exit(
                if msg.starts_with("arguments:")
                    || msg.starts_with("configuration")
                    || msg.contains("min-confidence")
                    || msg.contains("group-by")
                {
                    2
                } else {
                    1
                },
            );
        }
    }
}
