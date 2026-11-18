//! `clipstash`: thin CLI over the core stores. It opens the same SQLite
//! files as the app (WAL mode), so it works while the app is running.

use clap::{Parser, Subcommand};
use clipstash_core::snippets::CollectionExport;
use clipstash_core::*;
use clipstash_platform_macos as mac;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

/// Bundle ids of the app, release first.
const APP_BUNDLE_IDS: [&str; 2] = ["com.clipstash.app", "com.clipstash.app.dev"];

#[derive(Parser)]
#[command(name = "clipstash", version, about = "Clipboard history and snippets")]
struct Cli {
    /// Data directory (default: ~/Library/Application Support/clipstash, or $CLIPSTASH_DIR)
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List recent clips, most recently used first
    List {
        /// text, image or file
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Search clips and snippets (substring match)
    Search {
        query: String,
        /// all, text, image, file or snippet
        #[arg(long, default_value = "all")]
        filter: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print the full content of a clip
    Show { id: i64 },
    /// Put a clip back on the clipboard
    Copy {
        id: i64,
        /// Leave out the rich-text payload
        #[arg(long)]
        plain: bool,
    },
    /// Clear history: the last 5 minutes, the last 15 minutes, or everything
    Clear {
        #[arg(default_value = "5m")]
        scope: String,
    },
    /// Apply retention now
    Prune,
    /// Manage snippets
    Snippet {
        #[command(subcommand)]
        cmd: SnippetCmd,
    },
    /// Open the snippet library window in the app
    Library,
    /// Open the clipboard viewer in the app
    Viewer,
    /// Report pasteboard access state and permissions (M0 probe)
    Doctor {
        /// Also perform one content read and time it. On macOS 15.4+ this
        /// may raise the system paste alert.
        #[arg(long)]
        read: bool,
    },
    /// Write text to the pasteboard (testing aid)
    #[command(hide = true)]
    Pbwrite {
        text: String,
        /// Mark the text with org.nspasteboard.ConcealedType
        #[arg(long)]
        concealed: bool,
    },
}

#[derive(Subcommand)]
enum SnippetCmd {
    /// List snippets
    List {
        #[arg(long)]
        collection: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Add a snippet; the body comes from --body or stdin
    Add {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = snippets::DEFAULT_COLLECTION)]
        collection: String,
        #[arg(long)]
        keyword: Option<String>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Copy a snippet to the clipboard with placeholders resolved
    Copy { id: i64 },
    /// Export a collection as JSON (stdout, or -o FILE)
    Export {
        collection: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Import a collection JSON file ("-" for stdin)
    Import {
        file: PathBuf,
        /// Replace snippets that already exist with the same name
        #[arg(long)]
        overwrite: bool,
    },
}

extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

fn main() -> ExitCode {
    // Die quietly when piped into `head`, instead of panicking on EPIPE.
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    unsafe { signal(SIGPIPE, SIG_DFL) };
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("clipstash: {e}");
            ExitCode::FAILURE
        }
    }
}

type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn age(now: i64, then: i64) -> String {
    let s = ((now - then).max(0)) / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&flat, max)
}

fn print_clip(c: &Clip, now: i64) {
    println!(
        "{:>6}  {:<5}  {:>4}  {:<14}  {}",
        c.id,
        c.kind.as_str(),
        age(now, c.last_used),
        one_line(c.app_name.as_deref().unwrap_or("-"), 14),
        one_line(&c.preview, 80)
    );
}

fn print_snippet(s: &Snippet, collection: &str) {
    println!(
        "{:>6}  {:<12}  {:<8}  {:<24}  {}",
        s.id,
        one_line(collection, 12),
        s.keyword.as_deref().unwrap_or("-"),
        one_line(&s.name, 24),
        one_line(&s.body, 60)
    );
}

fn run(cli: Cli) -> AnyResult<()> {
    // Commands that do not touch the stores.
    match &cli.cmd {
        Cmd::Doctor { read } => return doctor(*read),
        Cmd::Pbwrite { text, concealed } => {
            if *concealed {
                mac::pasteboard::write_concealed_text(text)?;
            } else {
                mac::pasteboard::write(&Payload::text(text.clone()), true)?;
            }
            return Ok(());
        }
        Cmd::Library => return open_app("--library"),
        Cmd::Viewer => return open_app("--viewer"),
        _ => {}
    }

    let dir = cli.dir.unwrap_or_else(paths::data_dir);
    let (history, snips) = open_stores(&dir)?;
    let now = now_ms();

    match cli.cmd {
        Cmd::List { kind, limit, json } => {
            let kind = match kind {
                Some(k) => Some(ClipKind::parse(&k).ok_or("kind must be text, image or file")?),
                None => None,
            };
            let clips = history.list(kind, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&clips)?);
            } else {
                clips.iter().for_each(|c| print_clip(c, now));
            }
        }
        Cmd::Search { query, filter, limit, json } => {
            let filter = SearchFilter::parse(&filter).ok_or("filter must be all, text, image, file or snippet")?;
            let hits = search(&history, &snips, &query, filter, limit, now)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                let cols = snips.collections()?;
                for hit in &hits {
                    match hit {
                        SearchHit::Clip(c) => print_clip(c, now),
                        SearchHit::Snippet(s) => {
                            let name = cols.iter().find(|c| c.id == s.collection_id).map(|c| c.name.as_str());
                            println!(
                                "{:>6}  {:<5}  {:>4}  {:<14}  {}",
                                format!("s{}", s.id),
                                "snip",
                                format!("{}×", s.use_count),
                                one_line(name.unwrap_or("-"), 14),
                                one_line(&format!("★ {}: {}", s.name, s.body), 80)
                            );
                        }
                    }
                }
            }
        }
        Cmd::Show { id } => {
            let d = history.get(id)?.ok_or("no such clip")?;
            match d.clip.kind {
                ClipKind::Text => print!("{}", d.text.unwrap_or_default()),
                ClipKind::File => d.paths().iter().for_each(|p| println!("{p}")),
                ClipKind::Image => println!("{}", d.blob_path.map(|p| p.display().to_string()).unwrap_or_default()),
            }
        }
        Cmd::Copy { id, plain } => {
            let d = history.get(id)?.ok_or("no such clip")?;
            let payload = match d.clip.kind {
                ClipKind::Text => Payload::Text { text: d.text.clone().unwrap_or_default(), rich: d.rich.clone() },
                ClipKind::File => Payload::Files { paths: d.paths() },
                ClipKind::Image => Payload::Image { png: history.read_blob(id)? },
            };
            mac::pasteboard::write(&payload, plain)?;
            history.touch(id, now)?;
        }
        Cmd::Clear { scope } => {
            let scope = ClearScope::parse(&scope).ok_or("scope must be 5m, 15m or all")?;
            let n = history.clear(scope, now)?;
            println!("removed {n} clip{}", if n == 1 { "" } else { "s" });
        }
        Cmd::Prune => {
            let n = history.prune(&snips.settings()?, now)?;
            println!("pruned {n} clip{}", if n == 1 { "" } else { "s" });
        }
        Cmd::Snippet { cmd } => snippet(cmd, &history, &snips, now)?,
        Cmd::Doctor { .. } | Cmd::Pbwrite { .. } | Cmd::Library | Cmd::Viewer => unreachable!(),
    }
    Ok(())
}

fn snippet(cmd: SnippetCmd, history: &HistoryStore, snips: &SnippetStore, now: i64) -> AnyResult<()> {
    match cmd {
        SnippetCmd::List { collection, json } => {
            let cols = snips.collections()?;
            let id = match &collection {
                Some(name) => Some(snips.collection_by_name(name)?.ok_or("no such collection")?),
                None => None,
            };
            let list = snips.list(id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                for s in &list {
                    let name = cols.iter().find(|c| c.id == s.collection_id).map(|c| c.name.as_str());
                    print_snippet(s, name.unwrap_or("-"));
                }
            }
        }
        SnippetCmd::Add { name, collection, keyword, body } => {
            let body = match body {
                Some(b) => b,
                None => {
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            let collection_id = snips.ensure_collection(&collection)?;
            let s = snips.create(&SnippetDraft { collection_id, name, keyword, body }, now)?;
            println!("added snippet {}", s.id);
        }
        SnippetCmd::Copy { id } => {
            let s = snips.get(id)?.ok_or("no such snippet")?;
            let payload = if s.kind == "image" {
                Payload::Image { png: snips.blob(id)?.ok_or("snippet has no image")? }
            } else {
                let expanded = placeholder::expand(&s.body, chrono::Local::now().naive_local(), &|n| {
                    history.nth_text(n).ok().flatten()
                });
                Payload::text(expanded.text)
            };
            mac::pasteboard::write(&payload, true)?;
            snips.bump_use(id)?;
        }
        SnippetCmd::Export { collection, output } => {
            let id = snips.collection_by_name(&collection)?.ok_or("no such collection")?;
            let json = serde_json::to_string_pretty(&snips.export_collection(id)?)?;
            match output {
                Some(path) => std::fs::write(path, json + "\n")?,
                None => println!("{json}"),
            }
        }
        SnippetCmd::Import { file, overwrite } => {
            let raw = if file.as_os_str() == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(&file)?
            };
            let data: CollectionExport = serde_json::from_str(&raw)?;
            let mode = if overwrite { ImportMode::Overwrite } else { ImportMode::Skip };
            let r = snips.import_collection(&data, mode, now)?;
            println!("{}: {} created, {} updated", r.collection, r.created, r.updated);
            if !r.conflicts.is_empty() {
                println!("{} already exist and were left alone (use --overwrite to replace):", r.conflicts.len());
                r.conflicts.iter().for_each(|n| println!("  {n}"));
            }
        }
    }
    Ok(())
}

fn open_app(flag: &str) -> AnyResult<()> {
    // `open -n` starts a second instance; the app's single-instance guard
    // forwards the arguments to the running one and exits.
    for id in APP_BUNDLE_IDS {
        let ok = std::process::Command::new("/usr/bin/open")
            .args(["-n", "-b", id, "--args", flag])
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
    }
    Err("the clipstash app is not installed".into())
}

fn doctor(read: bool) -> AnyResult<()> {
    let (maj, min, patch) = mac::os_version();
    println!("macOS                 {maj}.{min}.{patch}");
    let t = std::time::Instant::now();
    let behavior = mac::pasteboard::access_behavior();
    println!("accessBehavior        {} ({:?})", behavior.as_str(), t.elapsed());
    println!("background capture    {}", if behavior.may_read() { "allowed" } else { "blocked: clipboard access needed" });
    let t = std::time::Instant::now();
    println!("changeCount           {} ({:?})", mac::pasteboard::change_count(), t.elapsed());
    let t = std::time::Instant::now();
    let types = mac::pasteboard::types();
    println!("types                 {} present ({:?})", types.len(), t.elapsed());
    for ty in &types {
        println!("                        {ty}");
    }
    println!("classified as         {}", mac::pasteboard::classify(&types).map(ClipKind::as_str).unwrap_or("nothing capturable"));
    println!("concealed marker      {}", filter::has_concealed_marker(&types));
    if read {
        let t = std::time::Instant::now();
        let text = mac::pasteboard::read_text();
        println!(
            "content read          {} ({:?})",
            match &text {
                Some(s) => format!("{} bytes of text", s.len()),
                None => "nil".to_string(),
            },
            t.elapsed()
        );
    }
    println!("accessibility         {}", if mac::permissions::accessibility_trusted() { "granted" } else { "not granted (copy-only fallback)" });
    println!("  settings list       {}", mac::permissions::accessibility_pane_name(maj));
    println!("secure input          {}", mac::permissions::secure_input_active());
    let front = mac::workspace::frontmost_app();
    println!(
        "frontmost app         {}",
        front.map(|a| format!("{} ({})", a.name.unwrap_or_default(), a.bundle_id.unwrap_or_default())).unwrap_or_default()
    );
    println!("data dir              {}", paths::data_dir().display());
    Ok(())
}
