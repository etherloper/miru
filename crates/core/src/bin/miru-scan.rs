//! Developer tool: scan folders into a library database and print what was found.
//!
//! Usage: miru-scan <library.db> [folder ...] [--search <query>]

use std::path::Path;
use std::process::ExitCode;

use miru_core::db::Db;
use miru_core::parse::ParsedMedia;
use miru_core::scan::scan_root;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let db_path = args
        .next()
        .ok_or("usage: miru-scan <library.db> [folder ...] [--search <query>]")?;
    let mut folders = Vec::new();
    let mut search = None;
    while let Some(arg) = args.next() {
        if arg == "--search" {
            search = Some(args.next().ok_or("--search needs a query")?);
        } else {
            folders.push(arg);
        }
    }

    let mut db = Db::open(Path::new(&db_path))?;
    for folder in &folders {
        let absolute = std::path::absolute(folder)?;
        db.add_root(absolute.to_str().ok_or("folder path is not UTF-8")?)?;
    }
    for root in db.roots()? {
        let r = scan_root(&mut db, &root)?;
        println!(
            "{}: +{} added, ~{} updated, ={} unchanged, -{} removed, {} errors",
            root.path, r.added, r.updated, r.unchanged, r.removed, r.errors
        );
    }
    db.set_parser_version(miru_core::parse::PARSER_VERSION)?;

    if let Some(query) = search {
        println!("\nSearch {query:?}:");
        for file in db.search(&query, 20)? {
            println!("  {}  ({})", describe(&file.media), file.path);
        }
        return Ok(());
    }

    println!("\nMovies:");
    for m in db.movies()? {
        match m.year {
            Some(year) => println!("  {} ({year})", m.title),
            None => println!("  {}", m.title),
        }
    }
    println!("\nShows:");
    for show in db.shows()? {
        println!(
            "  {} ({} seasons, {} episodes)",
            show.title, show.seasons, show.episodes
        );
        for ep in db.episodes(&show.title)? {
            println!("    {}", describe(&ep.media));
        }
    }
    Ok(())
}

fn describe(media: &ParsedMedia) -> String {
    match media {
        ParsedMedia::Movie {
            title,
            year: Some(y),
        } => format!("{title} ({y})"),
        ParsedMedia::Movie { title, year: None } => title.clone(),
        ParsedMedia::Episode {
            show,
            season,
            episode,
            episode_end,
            title,
        } => {
            let range = episode_end.map(|e| format!("-E{e:02}")).unwrap_or_default();
            let name = title
                .as_deref()
                .map(|t| format!(" {t}"))
                .unwrap_or_default();
            format!("{show} S{season:02}E{episode:02}{range}{name}")
        }
    }
}
