mod db;
mod disassemble;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use clap::builder::{
    IntoResettable, PossibleValue, PossibleValuesParser, TypedValueParser, ValueParser,
};
use clap::Parser;
use indicatif::ProgressBar;
use itertools::Itertools;
use patricia_tree::StringPatriciaMap;
use patternsleuth::image::Image;
use patternsleuth::resolvers::{resolvers, NamedResolver};

use patternsleuth::scanner::Xref;
use patternsleuth::symbols::Symbol;
use patternsleuth::{scanner::Pattern, PatternConfig, Resolution};

#[derive(Parser)]
enum Commands {
    Scan(CommandScan),
    Report(CommandReport),
    DiffReport(CommandDiffReport),
    Symbols(CommandSymbols),
    BuildIndex(CommandBuildIndex),
    ViewSymbol(CommandViewSymbol),
    AutoGen(CommandAutoGen),
}

fn parse_maybe_hex(s: &str) -> Result<usize> {
    Ok(s.strip_prefix("0x")
        .map(|s| usize::from_str_radix(s, 16))
        .unwrap_or_else(|| s.parse())?)
}

fn resolver_parser() -> impl IntoResettable<ValueParser> {
    fn parse_resolver(s: &str) -> Result<&'static NamedResolver> {
        resolvers()
            .find(|res| s == res.name)
            .context("Resolver not found")
    }
    fn possible_resolvers() -> Vec<PossibleValue> {
        resolvers().map(|r| r.name.into()).collect()
    }
    PossibleValuesParser::new(possible_resolvers()).map(|v| parse_resolver(&v).unwrap())
}

#[derive(Parser)]
struct CommandScan {
    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,

    /// A game process ID to attach to and scan
    #[arg(long)]
    pid: Vec<i32>,

    /// A path to a game's exe
    #[arg(long)]
    path: Vec<PathBuf>,

    /// A resolver to scan for (can be specified multiple times)
    #[arg(short, long, value_parser(resolver_parser()))]
    resolver: Vec<&'static NamedResolver>,

    /// Show disassembly context for each stage of every match (I recommend only using with
    /// aggressive filters)
    #[arg(short, long)]
    disassemble: bool,

    /// Show disassembly context for each matched address
    #[arg(long)]
    disassemble_merged: bool,

    /// A pattern to scan for (can be specified multiple times)
    #[arg(short, long, value_parser(|s: &_| Pattern::new(s)))]
    patterns: Vec<Pattern>,

    /// A path to a JSON pattern config file
    #[arg(long)]
    pattern_config: Option<PathBuf>,

    /// An xref to scan for (can be specified multiple times)
    #[arg(short, long, value_parser(|s: &str| parse_maybe_hex(s).map(Xref)))]
    xref: Vec<Xref>,

    /// Load and display symbols from PDBs when available (can be slow)
    #[arg(long)]
    symbols: bool,

    /// Skip parsing of exception table
    #[arg(long)]
    skip_exceptions: bool,

    /// Show scan summary
    #[arg(long)]
    summary: bool,

    /// Show scan progress
    #[arg(long)]
    progress: bool,
}

#[derive(Parser)]
struct CommandReport {
    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,

    /// A resolver to scan for (can be specified multiple times)
    #[arg(short, long, value_parser(resolver_parser()))]
    resolver: Vec<&'static NamedResolver>,
}

#[derive(Parser)]
struct CommandDiffReport {
    /// Path to first report
    a: PathBuf,

    /// Path to second report
    b: PathBuf,
}

#[derive(Parser)]
struct CommandSymbols {
    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,

    #[arg(short, long)]
    symbol: Vec<regex::Regex>,
}

#[derive(Parser)]
struct CommandBuildIndex {
    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,
}

#[derive(Parser)]
struct CommandReadIndex {}

#[derive(Parser)]
struct CommandSearchIndex {
    #[arg()]
    symbol: String,
}

#[derive(Debug, Clone)]
struct FunctionSpec {
    path: String,
    start: usize,
    end: usize,
}
impl FromStr for FunctionSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut iter = s.split(':');
        if let (Some(path), Some(start), Some(end), None) =
            (iter.next(), iter.next(), iter.next(), iter.next())
        {
            Ok(FunctionSpec {
                path: path.to_owned(),
                start: parse_maybe_hex(start)?,
                end: parse_maybe_hex(end)?,
            })
        } else {
            bail!("failed to parse function definition: expected format <path.exe>:<start>:<end>")
        }
    }
}

#[derive(Parser)]
struct CommandViewSymbol {
    #[arg(short, long)]
    symbol: Vec<String>,

    #[arg(short, long)]
    function: Vec<FunctionSpec>,

    #[arg(short, long, value_parser(resolver_parser()))]
    resolver: Vec<&'static NamedResolver>,

    /// Whether to show symbols in function disassembly
    #[arg(long)]
    show_symbols: bool,
}

#[derive(Parser)]
struct CommandAutoGen {}

fn find_ext<P: AsRef<Path>, E: AsRef<str>>(dir: P, ext: &[E]) -> Result<Option<PathBuf>> {
    for f in fs::read_dir(dir)? {
        let f = f?.path();
        if f.is_file()
            && f.extension()
                .and_then(std::ffi::OsStr::to_str)
                .map(|e| ext.iter().any(|m| m.as_ref().eq_ignore_ascii_case(e)))
                .unwrap_or_default()
        {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

fn main() -> Result<()> {
    use tracing_subscriber::{fmt, fmt::format::FmtSpan, EnvFilter};

    fmt()
        .compact()
        .with_level(true)
        .with_target(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_env_filter(EnvFilter::builder().from_env_lossy())
        .init();

    match Commands::parse() {
        Commands::Scan(command) => scan(command),
        Commands::Report(command) => report(command),
        Commands::DiffReport(command) => diff_report(command),
        Commands::Symbols(command) => symbols(command),
        Commands::BuildIndex(command) => db::build(command),
        Commands::ViewSymbol(command) => db::view(command),
        Commands::AutoGen(command) => db::auto_gen(command),
    }
}

// TODO remove, only used for patterns/xrefs from CLI
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct Sig(String);
impl std::fmt::Display for Sig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

fn scan(command: CommandScan) -> Result<()> {
    let include_default = command.patterns.is_empty() && command.xref.is_empty();
    // TODO warn if empty?
    let patterns = command
        .patterns
        .into_iter()
        .enumerate()
        .map(|(i, p)| PatternConfig::new(Sig("arg".to_string()), format!("pattern {i}"), None, p))
        .chain(command.xref.into_iter().enumerate().map(|(i, p)| {
            PatternConfig::xref(Sig("arg".to_string()), format!("xref {i}"), None, p)
        }))
        .chain(command.pattern_config.into_iter().flat_map(|path| {
            let file = std::fs::read_to_string(path).unwrap();
            let config: HashMap<String, Vec<String>> = serde_json::from_str(&file).unwrap();

            config.into_iter().flat_map(|(symbol, patterns)| {
                patterns.into_iter().enumerate().map(move |(i, p)| {
                    PatternConfig::new(
                        Sig(format!("file {symbol}")),
                        format!("#{i} {symbol}"),
                        None,
                        Pattern::new(p).unwrap(),
                    )
                })
            })
        }))
        .collect_vec();

    let resolvers = if command.resolver.is_empty() && include_default {
        resolvers().collect::<Vec<_>>()
    } else {
        command.resolver
    };
    let dyn_resolvers = resolvers.iter().map(|res| res.getter).collect::<Vec<_>>();

    let sigs = patterns
        .iter()
        .map(|p| p.sig.clone())
        .collect::<HashSet<_>>();

    let mut games: HashSet<String> = Default::default();

    let mut all: HashMap<(String, (&Sig, &String)), Vec<Resolution>> = HashMap::new();
    let mut all_resolutions: HashMap<String, _> = Default::default();

    use colored::Colorize;
    use indicatif::ProgressIterator;
    use itertools::join;
    use prettytable::{format, row, Cell, Row, Table};

    enum Output {
        Stdout,
        Progress(ProgressBar),
    }

    impl Output {
        fn println<M: AsRef<str>>(&self, msg: M) {
            match self {
                Output::Stdout => println!("{}", msg.as_ref()),
                Output::Progress(progress) => progress.println(msg),
            }
        }
    }

    let mut games_vec = vec![];

    for pid in command.pid {
        games_vec.push(GameEntry::Process(GameProcessEntry { pid }));
    }
    for path in command.path {
        games_vec.push(GameEntry::File(GameFileEntry {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            exe_path: path,
        }));
    }
    if games_vec.is_empty() {
        games_vec.extend(get_games(command.game)?.into_iter().map(GameEntry::File));
    }

    let (output, iter): (_, Box<dyn Iterator<Item = _>>) = if command.progress {
        let progress = ProgressBar::new(games_vec.len() as u64);
        (
            Output::Progress(progress.clone()),
            Box::new(games_vec.iter().progress_with(progress)),
        )
    } else {
        (Output::Stdout, Box::new(games_vec.iter()))
    };

    for game in iter {
        #[allow(unused_assignments)]
        let mut bin_data = None;

        let (name, exe) = match game {
            GameEntry::File(GameFileEntry { name, exe_path }) => {
                output.println(format!("{:?} {:?}", name, exe_path.display()));

                bin_data = Some(fs::read(exe_path)?);

                (Cow::Borrowed(name), {
                    let bin_data = bin_data.as_ref().unwrap();
                    let builder = Image::builder().functions(!command.skip_exceptions);
                    let exe = if command.symbols {
                        builder.symbols(exe_path).build(bin_data)
                    } else {
                        builder.build(bin_data)
                    };
                    match exe {
                        Ok(exe) => exe,
                        Err(err) => {
                            output.println(format!("err reading {}: {}", exe_path.display(), err));
                            continue;
                        }
                    }
                })
            }
            GameEntry::Process(GameProcessEntry { pid }) => {
                output.println(format!("PID={pid}"));

                (
                    Cow::Owned(format!("PID={pid}")),
                    patternsleuth::process::external::read_image_from_pid(*pid)?,
                )
            }
        };

        games.insert(name.to_string());

        let scan = exe.scan(&patterns)?;

        // group results by Sig
        let folded_scans = scan
            .results
            .iter()
            .map(|(config, m)| (&config.sig, (config, m)))
            .fold(HashMap::new(), |mut map: HashMap<_, Vec<_>>, (k, v)| {
                map.entry(k).or_default().push(v);
                map
            });

        let mut table = Table::new();
        table.set_titles(row!["sig", "offline scan"]);

        for sig in &sigs {
            let mut cells = vec![];
            cells.push(Cell::new(&sig.to_string()));

            if let Some(sig_scans) = folded_scans.get(&sig) {
                if command.disassemble {
                    let mut table = Table::new();
                    table.set_format(*format::consts::FORMAT_NO_BORDER);
                    for m in sig_scans.iter() {
                        let mut cells = vec![];
                        cells.push(Cell::new(&format!(
                            "{}\n{}",
                            m.0.name,
                            disassemble::disassemble(
                                &exe,
                                m.1.address,
                                m.0.scan.scan_type.get_pattern()
                            )
                        )));
                        table.add_row(Row::new(cells));
                    }
                    cells.push(Cell::new(&table.to_string()));
                } else if command.disassemble_merged {
                    cells.push(Cell::new({
                        let cells = sig_scans
                            .iter()
                            .fold(
                                HashMap::<&Resolution, HashMap<&str, usize>>::new(),
                                |mut map, m| {
                                    *map.entry(m.1).or_default().entry(&m.0.name).or_default() += 1;
                                    map
                                },
                            )
                            .iter()
                            // sort by pattern name, then match address
                            .sorted_by_key(|&data| data.0)
                            .map(|(m, counts)| {
                                let dis = disassemble::disassemble(&exe, m.address, None);

                                let mut lines = vec![];
                                for (name, count) in counts.iter().sorted_by_key(|e| e.0) {
                                    let count = if *count > 1 {
                                        format!(" (x{count})")
                                    } else {
                                        "".to_string()
                                    };

                                    lines.push(format!("{name:?}{count}").normal().to_string());
                                }
                                lines.push(dis);

                                Cell::new(&join(lines, "\n"))
                            })
                            .collect::<Vec<_>>();

                        let mut table = Table::new();
                        table.set_format(*format::consts::FORMAT_NO_BORDER);

                        table.add_row(Row::new(cells));
                        table.to_string()
                    }.as_str()));
                } else {
                    cells.push(Cell::new({
                        let mut lines = sig_scans
                            .iter()
                            // group and count matches by (pattern name, address)
                            .fold(
                                HashMap::<(&String, &Resolution), usize>::new(),
                                |mut map, m| {
                                    *map.entry((&m.0.name, m.1)).or_default() += 1;
                                    map
                                },
                            )
                            .iter()
                            // sort by pattern name, then match address
                            .sorted_by_key(|&data| data.0)
                            .map(|(m, count)| {
                                // add count indicator if more than 1
                                let count = if *count > 1 {
                                    format!(" (x{count})")
                                } else {
                                    "".to_string()
                                };

                                (
                                    format!("{:016x} {:?}{}", m.1.address, m.0, count)
                                        .normal()
                                        .to_string(),
                                    exe.symbols
                                        .as_ref()
                                        .and_then(|symbols| symbols.get(&m.1.address)),
                                )
                            })
                            .collect::<Vec<_>>();
                        let max_len = lines.iter().map(|(line, _)| line.len()).max();
                        for (line, symbol) in &mut lines {
                            if let Some(symbol) = symbol {
                                line.push_str(&format!(
                                    "{}{}",
                                    " ".repeat(1 + max_len.unwrap() - line.len()),
                                    symbol.name.bright_yellow()
                                ));
                            }
                        }
                        join(lines.iter().map(|(line, _)| line), "\n").to_string()
                    }.as_str()));
                }
            } else {
                #[allow(clippy::unnecessary_to_owned)]
                cells.push(Cell::new(&"not found".red().to_string()));
            }

            table.add_row(Row::new(cells));
        }

        let game_name = match game {
            GameEntry::File(GameFileEntry { name, .. }) => name.clone(),
            GameEntry::Process(GameProcessEntry { pid }) => format!("pid={pid}"),
        };

        let resolution = tracing::info_span!("scan", game = game_name)
            .in_scope(|| exe.resolve_many(&dyn_resolvers));

        for (resolver, resolution) in resolvers.iter().zip(&resolution) {
            table.add_row(Row::new(
                [
                    Cell::new(resolver.name),
                    match resolution {
                        Ok(res) => Cell::new(&format!("{res:#x?}")),
                        Err(err) =>
                        {
                            #[allow(clippy::unnecessary_to_owned)]
                            Cell::new(&format!("{err:x?}").red().to_string())
                        }
                    },
                ]
                .to_vec(),
            ));
        }

        if !resolution.is_empty() {
            all_resolutions.insert(name.to_string(), resolution);
        }

        output.println(table.to_string());

        // fold current game scans into summary scans
        scan.results.into_iter().fold(&mut all, |map, m| {
            map.entry((name.to_string(), (&m.0.sig, &m.0.name)))
                .or_default()
                .push(m.1);
            map
        });
    }

    // force any progress output to be dropped
    let output = Output::Stdout;

    if command.summary {
        #[derive(Debug, Default)]
        struct Summary {
            matches: usize,
        }
        impl Summary {
            fn format(&self) -> String {
                if self.matches == 0 {
                    "none".to_owned()
                } else {
                    format!("M={}", self.matches)
                }
            }
        }

        let mut summary = Table::new();
        let title_strs: Vec<String> = ["".into(), "unqiue addresses".into()]
            .into_iter()
            .chain(
                patterns
                    .iter()
                    .map(|conf| format!("{:?}({})", conf.sig, conf.name)),
            )
            .chain(resolvers.iter().map(|r| r.name.to_string()))
            .collect();
        summary.set_titles(Row::new(title_strs.iter().map(|s| Cell::new(s)).collect()));
        let mut totals = patterns.iter().map(|_| Summary::default()).collect_vec();

        let mut no_matches = 0;
        let mut one_match = 0;
        let mut gt_one_match = 0;

        for game in games.iter().sorted() {
            let mut row = vec![Cell::new(game)];

            let mut matched_addresses = HashSet::new();

            let summaries: Vec<Summary> = patterns
                .iter()
                .map(|conf| {
                    let res = all.get(&(game.to_string(), (&conf.sig, &conf.name)));
                    if let Some(res) = res {
                        for res in res {
                            matched_addresses.insert(res.address);
                        }
                        Summary { matches: res.len() }
                    } else {
                        Summary { matches: 0 }
                    }
                })
                .collect();

            for (i, s) in summaries.iter().enumerate() {
                if s.matches > 0 {
                    totals[i].matches += 1;
                }
            }

            match matched_addresses.len() {
                0 => {
                    no_matches += 1;
                }
                1 => {
                    one_match += 1;
                }
                _ => {
                    gt_one_match += 1;
                }
            }

            row.push(Cell::new(&format!("unique={}", matched_addresses.len())));

            let cell_strs: Vec<String> = summaries.iter().map(Summary::format).collect();
            row.extend(cell_strs.iter().map(|s| Cell::new(s)));

            if let Some(res) = all_resolutions.get(game) {
                for res in res {
                    match res {
                        Ok(res) => row.push(Cell::new(&format!("{res:x?}"))),
                        Err(err) => {
                            #[allow(clippy::unnecessary_to_owned)]
                            row.push(Cell::new(&format!("{err:x?}").red().to_string()));
                        }
                    }
                }
            }

            summary.add_row(Row::new(row));
        }

        let total_strs = [
            format!("total={}", games.len()),
            format!("0={no_matches} 1={one_match} >1={gt_one_match}"),
        ]
        .into_iter()
        .chain(totals.iter().map(Summary::format))
        .chain(resolvers.iter().enumerate().map(|(i, _)| {
            let ok = all_resolutions.values().filter(|r| r[i].is_ok()).count();
            format!(
                "Ok={ok}/{} ({:.2}%)",
                games.len(),
                100. * ok as f64 / games.len() as f64
            )
        }))
        .collect_vec();
        summary.add_row(Row::new(
            total_strs.iter().map(|s| Cell::new(s)).collect_vec(),
        ));

        //let games: HashSet<String> = all.keys().map(|(game, _)| game).cloned().collect();
        //println!("{:#?}", all);

        output.println(summary.to_string());
    }

    Ok(())
}

fn report(command: CommandReport) -> Result<()> {
    use rayon::prelude::*;

    fn load_game(path: impl AsRef<Path>, data: &mut Vec<u8>) -> Result<Image<'_>> {
        use std::io::Read;
        data.clear();
        fs::File::open(path)?.read_to_end(data)?;
        Image::builder().build(data)
    }

    let resolvers = command
        .resolver
        .iter()
        .map(|res| res.getter)
        .collect::<Vec<_>>();

    let time = time::OffsetDateTime::now_local()?.format(time::macros::format_description!(
        "[year]-[month]-[day]_[hour]-[minute]-[second]"
    ))?;

    let games = get_games(command.game)?;

    let results = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));

    let progress = ProgressBar::new(games.len() as u64);
    games.into_par_iter().try_for_each(|game| -> Result<()> {
        progress.println(format!("{:?} {:?}", game.name, game.exe_path.display()));

        let mut data = vec![];
        let exe = match load_game(&game.exe_path, &mut data) {
            Ok(exe) => exe,
            Err(err) => {
                progress.println(format!("err reading {}: {}", game.exe_path.display(), err));
                progress.inc(1);
                return Ok(());
            }
        };

        let resolution = exe.resolve_many(&resolvers);

        let map = command
            .resolver
            .iter()
            .zip(resolution)
            .map(|(resolver, resolution)| (resolver.name, resolution))
            .collect::<BTreeMap<_, _>>();
        results.lock().unwrap().insert(game.name, map);

        progress.inc(1);

        Ok(())
    })?;

    fs::create_dir_all("reports")?;
    fs::write(
        format!(
            "reports/{}{}{}.json",
            time,
            option_env!("GIT_HASH")
                .map(|hash| format!("-{}", &hash[..10]))
                .unwrap_or_default(),
            option_env!("GIT_DIRTY")
                .map(|_| "-dirty")
                .unwrap_or_default(),
        ),
        serde_json::to_vec(
            &std::sync::Arc::try_unwrap(results)
                .unwrap()
                .into_inner()
                .unwrap(),
        )
        .unwrap(),
    )?;

    Ok(())
}
fn diff_report(command: CommandDiffReport) -> Result<()> {
    use colored::Colorize;
    use patternsleuth::resolvers::{Resolution, ResolveError};
    use prettytable::{Cell, Row, Table};
    type Report = BTreeMap<String, BTreeMap<String, Result<Box<dyn Resolution>, ResolveError>>>;

    let a: Report = serde_json::from_slice(&fs::read(command.a)?)?;
    let b: Report = serde_json::from_slice(&fs::read(command.b)?)?;

    let mut games_only_in_a = vec![];
    let mut games_only_in_b = vec![];

    type Res<'r> = Result<&'r Box<dyn Resolution + 'static>, &'r ResolveError>;
    let mut diffs: BTreeMap<&str, BTreeMap<&str, (Res, Res)>> = Default::default();

    for game in a.keys().chain(b.keys()).unique() {
        let game_a = a.get(game);
        let game_b = b.get(game);
        if game_a.is_none() {
            games_only_in_b.push(game);
        }
        if game_b.is_none() {
            games_only_in_a.push(game);
        }
        if let (Some(game_a), Some(game_b)) = (game_a, game_b) {
            for res in game_a.keys().chain(game_b.keys()).unique() {
                if let (Some(res_a), Some(res_b)) = (game_a.get(res), game_b.get(res)) {
                    diffs
                        .entry(res)
                        .or_default()
                        .insert(game, (res_a.as_ref(), res_b.as_ref()));
                } else {
                    // TODO warn if mismatched set of resolvers
                }
            }
        }
    }

    dbg!(games_only_in_a);
    dbg!(games_only_in_b);

    fn local<I, O, F: FnOnce(I) -> O>(i: I, f: F) -> O {
        f(i)
    }

    fn format_res(res: Result<&dyn Resolution, &ResolveError>) -> String {
        local(format!("{res:x?}").bold(), |s| match res {
            Ok(_) => s,
            Err(_) => s.red(),
        })
        .to_string()
    }

    fn format_percent_diff(percent_diff: f32) -> String {
        local(
            format!("{percent_diff:+.2?}%").bold(),
            |s| match percent_diff {
                f if f < 0. => s.red(),
                f if f > 0. => s.green(),
                _ => s,
            },
        )
        .to_string()
    }

    struct ResEntry {
        ok_diff: usize,
        percent_a: f32,
        percent_b: f32,
        percent_diff: f32,
    }

    let mut results = vec![];

    for (res, entries) in diffs {
        let mut table = Table::new();

        let total = entries.len();
        let ok_a = entries.values().filter(|res| res.0.is_ok()).count();
        let ok_b = entries.values().filter(|res| res.1.is_ok()).count();
        let diff = entries
            .iter()
            .filter(|(_, (a, b))| a.ok() != b.ok())
            .collect::<Vec<_>>();
        let ok_diff = diff
            .iter()
            .filter(|(_, pair)| matches!(pair, (Ok(a), Ok(b)) if a != b))
            .count();

        let percent_a = ok_a as f32 / total as f32 * 100.;
        let percent_b = ok_b as f32 / total as f32 * 100.;
        let percent_diff = percent_b - percent_a;

        results.push((
            res,
            ResEntry {
                ok_diff,
                percent_a,
                percent_b,
                percent_diff,
            },
        ));

        if diff.is_empty() {
            continue;
        }

        let score = format_percent_diff(percent_diff);
        let changed = if ok_diff == 0 {
            "".to_string()
        } else {
            format!("{ok_diff} changed").yellow().bold().to_string()
        };
        let title = format!(
            "{res} - {ok_a}/{total} ({percent_a:.2}%) => {ok_b}/{total} ({percent_b:.2}%): {score} {changed}"
        );
        table.set_titles(Row::new(vec![Cell::new(&title).with_hspan(3)]));

        for (game, (res_a, res_b)) in diff {
            table.add_row(Row::new(vec![
                Cell::new(game),
                Cell::new(&format_res(res_a.map(|ok| ok.as_ref()))),
                Cell::new(&format_res(res_b.map(|ok| ok.as_ref()))),
            ]));
        }

        table.printstd();
    }

    let mut table = Table::new();
    table.set_titles(Row::new(vec![
        Cell::new("resolver"),
        Cell::new("a"),
        Cell::new("b"),
        Cell::new("increase"),
        Cell::new("changed"),
    ]));
    for (res, entry) in results {
        table.add_row(Row::new(vec![
            Cell::new(res),
            Cell::new(&format!("{:.2}%", entry.percent_a)),
            Cell::new(&format!("{:.2}%", entry.percent_b)),
            Cell::new(&format_percent_diff(entry.percent_diff)),
            Cell::new(
                &local(format!("{}", entry.ok_diff), |s| match entry.ok_diff {
                    0 => s.normal(),
                    _ => s.yellow().bold(),
                })
                .to_string(),
            ),
        ]));
    }
    table.printstd();

    Ok(())
}

fn symbols(command: CommandSymbols) -> Result<()> {
    let re = &command.symbol;
    let filter = |sym: &Symbol| re.iter().any(|re| re.is_match(&sym.name));

    use prettytable::{Cell, Row, Table};

    let mut cells = vec![];

    for GameFileEntry { name, exe_path } in get_games(command.game)? {
        if !exe_path.with_extension("pdb").exists() && !exe_path.with_extension("sym").exists() {
            continue;
        }

        println!("{:?} {:?}", name, exe_path.display());
        let bin_data = fs::read(&exe_path)?;
        let exe = match Image::builder()
            .functions(true)
            .symbols(&exe_path)
            .build(&bin_data)
        {
            Ok(exe) => exe,
            Err(err) => {
                println!("err reading {}: {}", exe_path.display(), err);
                continue;
            }
        };

        for (address, sym) in exe.symbols.as_ref().unwrap() {
            if filter(sym) {
                if let Ok(Some(full_range)) = exe.get_root_function_range(*address) {
                    cells.push((
                        sym.clone(),
                        disassemble::disassemble_range(&exe, full_range),
                    ));
                } else {
                    println!("{:016x} [NO EXCEPT] {}", address, sym.name);
                }
            }
        }
    }

    let mut table = Table::new();
    table.set_titles(cells.iter().map(|c| c.0.name.clone()).collect());
    table.add_row(Row::new(
        cells.into_iter().map(|c| Cell::new(&c.1)).collect(),
    ));
    table.printstd();

    Ok(())
}

enum GameEntry {
    File(GameFileEntry),
    Process(GameProcessEntry),
}

struct GameFileEntry {
    name: String,
    exe_path: PathBuf,
}

struct GameProcessEntry {
    pid: i32,
}

fn get_games(filter: impl AsRef<[String]>) -> Result<Vec<GameFileEntry>> {
    let games_filter = filter
        .as_ref()
        .iter()
        .map(|g| {
            Ok(globset::GlobBuilder::new(g)
                .case_insensitive(true)
                .build()?
                .compile_matcher())
        })
        .collect::<Result<Vec<_>>>()?;
    //eprintln!("{}", std::env::current_dir().unwrap().to_str().unwrap());
    fs::read_dir("games")?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(|entry| -> Result<Option<(String, PathBuf)>> {
            //eprintln!("FSOK!");
            let dir_name = entry.file_name();
            let name = dir_name.to_string_lossy().to_string();
            if !(games_filter.is_empty() || games_filter.iter().any(|g| g.is_match(&name))) {
                return Ok(None);
            }

            let Some(exe_path) = find_ext(entry.path(), &["exe", "elf", "dmp"])
                .transpose()
                .transpose()?
            else {
                return Ok(None);
            };
            Ok(Some((name, exe_path)))
        })
        .filter_map(|r| r.transpose())
        .collect::<Result<Vec<(String, _)>>>()
        .map(|entries| {
            sample_order(entries, 3)
                .into_iter()
                .map(|(name, exe_path)| GameFileEntry { name, exe_path })
                .collect::<Vec<GameFileEntry>>()
        })
}

/// Distribute pairs such that unique prefixes are encountered early
/// e.g.
/// 7_a 8_a 9_a 7_b 7_c 7_d 8_b 8_c 9_b
fn sample_order<V>(entries: Vec<(String, V)>, prefix_size: usize) -> Vec<(String, V)> {
    let mut trie = StringPatriciaMap::from_iter(entries);
    let mut len = 1;
    let mut result = vec![];
    while !trie.is_empty() {
        let mut prefixes = HashSet::new();
        for (k, _v) in trie.iter() {
            if k.chars().count() >= len {
                prefixes.insert(k.chars().take(len).collect::<String>());
            }
        }
        for p in prefixes.iter().sorted() {
            let take = trie
                .iter_prefix(p)
                .take(prefix_size)
                .map(|(k, _v)| k)
                .collect_vec();
            for k in take {
                let v = trie.remove(k.clone()).unwrap();
                result.push((k, v));
            }
        }
        len += 1;
    }
    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sample_cont() {
        let entries = ["aa", "ba", "ca", "ab", "ac", "bc"]
            .iter()
            .map(|k| (k.to_string(), ()))
            .collect_vec();
        let ordered = sample_order(entries.clone(), 1);
        assert_eq!(entries, ordered);
    }
}
