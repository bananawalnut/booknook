use booknook_native::Desk;
use std::io::Read;
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return Err("Usage: booknook DESK_DIR snapshot|command|gates|import-content BOOK_ID FILE|export FILE|restore FILE".into());
    }
    let desk = Desk::new(args[1].clone())?;
    match args[2].as_str() {
        "snapshot" => println!("{}", desk.snapshot()?),
        "command" => {
            let mut raw = String::new();
            std::io::stdin()
                .take(32 * 1024 * 1024)
                .read_to_string(&mut raw)?;
            println!("{}", desk.execute(raw)?);
        }
        "gates" => println!("{}", desk.release_gates()?),
        "import-content" if args.len() == 5 => {
            println!("{}", desk.import_content(args[3].clone(), args[4].clone())?)
        }
        "export" if args.len() == 4 => desk.export_archive(args[3].clone())?,
        "restore" if args.len() == 4 => println!("{}", desk.restore_archive(args[3].clone())?),
        _ => return Err("Invalid command or arguments".into()),
    }
    Ok(())
}
