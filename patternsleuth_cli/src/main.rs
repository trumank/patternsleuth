use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use itertools::Itertools;
use object::{Object, ObjectSection};
use patternsleuth::patterns::resolve_self;
use patternsleuth::Executable;

use patternsleuth::scanner::Xref;
use patternsleuth::{
    patterns::{get_patterns, Sig},
    scanner::Pattern,
    PatternConfig, Resolution, ResolutionAction, ResolutionType, ResolveContext, ResolveStages,
    Scan,
};

mod sift4;

#[derive(Parser)]
enum Commands {
    Scan(CommandScan),
    Symbols(CommandSymbols),
    BuildIndex(CommandBuildIndex),
    ReadIndex(CommandReadIndex),
    SearchIndex(CommandSearchIndex),
    ListIndex(CommandListIndex),
    ViewSymbol(CommandViewSymbol),
    BruteForce(CommandBruteForce),
}

fn parse_maybe_hex(s: &str) -> Result<usize> {
    Ok(s.strip_prefix("0x")
        .map(|s| usize::from_str_radix(s, 16))
        .unwrap_or_else(|| s.parse())?)
}

#[derive(Parser)]
struct CommandScan {
    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,

    /// A signature to scan for (can be specified multiple times). Scans for all signatures if omitted
    #[arg(short, long, group = "scan")]
    signature: Vec<Sig>,

    /// Show disassembly context for each stage of every match (I recommend only using with
    /// aggressive filters)
    #[arg(short, long)]
    disassemble: bool,

    /// A pattern to scan for (can be specified multiple times)
    #[arg(short, long, group = "scan", value_parser(|s: &_| Pattern::new(s)))]
    patterns: Vec<Pattern>,

    /// An xref to scan for (can be specified multiple times)
    #[arg(short, long, group = "scan", value_parser(|s: &str| parse_maybe_hex(s).map(Xref)))]
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

#[derive(Parser)]
struct CommandViewSymbol {
    #[arg()]
    symbol: String,

    /// Whether to show symbols in function disassembly
    #[arg(long)]
    show_symbols: bool,
}

#[derive(Parser)]
struct CommandListIndex {}

#[derive(Parser)]
struct CommandBruteForce {}

mod disassemble {
    use std::ops::Range;

    use super::*;
    use colored::{ColoredString, Colorize};
    use iced_x86::{
        Decoder, DecoderOptions, Formatter, FormatterOutput, FormatterTextKind, IntelFormatter,
        OpKind,
    };

    #[derive(Default)]
    struct Output {
        pub buffer: String,
    }

    impl FormatterOutput for Output {
        fn write(&mut self, text: &str, kind: FormatterTextKind) {
            #[allow(clippy::unnecessary_to_owned)]
            self.buffer.push_str(&get_color(text, kind).to_string());
        }
    }

    pub(crate) fn disassemble(
        exe: &Executable,
        address: usize,
        pattern: Option<&Pattern>,
    ) -> String {
        let context = 20; // number of instructions before and after
        let max_inst = 16; // max size of x86 instruction in bytes

        let mut output = Output::default();

        if let Some(section) = exe.memory.get_section_containing(address) {
            output.buffer.push_str(&format!(
                "{:016x}\n{:016x} - {:016x} = {}\n",
                address,
                section.address,
                section.address + section.data.len(),
                section.name,
            ));

            let (is_fn, data, start_address) = if let Some(f) = exe.get_function(address) {
                output.buffer.push_str(&format!(
                    "{:016x} - {:016x} = function\n",
                    f.range.start, f.range.end
                ));
                if let Some(symbols) = &exe.symbols {
                    if let Some(symbol) = symbols.get(&f.range.start) {
                        #[allow(clippy::unnecessary_to_owned)]
                        output
                            .buffer
                            .push_str(&format!("{}\n", symbol).bright_yellow().to_string());
                    }
                }
                let data =
                    &section.data[f.range.start - section.address..f.range.end - section.address];
                let start_address = f.range.start as u64;
                (true, data, start_address)
            } else {
                output.buffer.push_str("no function");

                let data = &section.data[(address - context * max_inst)
                    .saturating_sub(section.address)
                    ..(address + context * max_inst).saturating_sub(section.address)];
                let start_address = (address - context * max_inst) as u64;
                (false, data, start_address)
            };

            output.buffer.push('\n');

            let mut decoder = Decoder::with_ip(64, data, start_address, DecoderOptions::NONE);

            let instructions = decoder.iter().collect::<Vec<_>>();
            let instructions = if let Some((middle, _)) = (!is_fn)
                .then(|| {
                    instructions
                        .iter()
                        .enumerate()
                        .find(|(_, inst)| inst.ip() >= address as u64)
                })
                .flatten()
            {
                instructions
                    .into_iter()
                    .skip(middle - context)
                    .take(context * 2 + 1)
                    .collect::<Vec<_>>()
            } else {
                instructions
            };

            let mut formatter = IntelFormatter::new();
            formatter.options_mut().set_first_operand_char_index(8);
            for instruction in instructions {
                let ip = format!("{:016x}", instruction.ip());
                if (instruction.ip()..instruction.ip() + instruction.len() as u64)
                    .contains(&(address as u64))
                {
                    #[allow(clippy::unnecessary_to_owned)]
                    output.buffer.push_str(&ip.reversed().to_string());
                } else {
                    output.buffer.push_str(&ip);
                }
                output.buffer.push_str(":  ");

                let index = (instruction.ip() - start_address) as usize;
                for (i, b) in data[index..index + instruction.len()].iter().enumerate() {
                    let highlight = pattern
                        .and_then(|p| -> Option<bool> {
                            let offset =
                                (instruction.ip() as usize) - address + i + p.custom_offset;
                            Some(*p.mask.get(offset)? != 0)
                        })
                        .unwrap_or_default();
                    let s = format!("{:02x}", b);
                    let mut colored = if highlight {
                        s.bright_white()
                    } else {
                        s.bright_black()
                    };
                    if instruction
                        .ip()
                        .checked_add(i as u64)
                        .map(|a| a == address as u64)
                        .unwrap_or_default()
                    {
                        colored = colored.reversed();
                    }
                    #[allow(clippy::unnecessary_to_owned)]
                    output.buffer.push_str(&colored.to_string());
                    output.buffer.push(' ');
                }

                for _ in 0..8usize.saturating_sub(instruction.len()) {
                    output.buffer.push_str("   ");
                }

                formatter.format(&instruction, &mut output);
                output.buffer.push('\n');
            }
        } else {
            output
                .buffer
                .push_str(&format!("{:016x}\nno section", address));
        }
        output.buffer
    }

    pub(crate) fn disassemble_range(exe: &Executable, range: Range<usize>) -> String {
        let address = range.start;
        let mut output = Output::default();

        if let Some(section) = exe.memory.get_section_containing(address) {
            let data = &section.data[range.start - section.address..range.end - section.address];

            output.buffer.push_str(&format!(
                "{:016x}\n{:016x} - {:016x} = {}\n",
                address,
                section.address,
                section.address + section.data.len(),
                section.name,
            ));

            if let Some(f) = exe.get_function(address) {
                output.buffer.push_str(&format!(
                    "{:016x} - {:016x} = function\n",
                    f.range.start, f.range.end
                ));
                if let Some(symbols) = &exe.symbols {
                    if let Some(symbol) = symbols.get(&f.range.start) {
                        #[allow(clippy::unnecessary_to_owned)]
                        output
                            .buffer
                            .push_str(&format!("{}\n", symbol).bright_yellow().to_string());
                    }
                }
            } else {
                output.buffer.push_str("no function");
            }

            output.buffer.push('\n');

            let mut decoder = Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE);

            let instructions = decoder.iter().collect::<Vec<_>>();

            let mut formatter = IntelFormatter::new();
            formatter.options_mut().set_first_operand_char_index(8);
            for instruction in instructions {
                let ip = format!("{:016x}", instruction.ip());
                output.buffer.push_str(&ip);
                output.buffer.push_str(":  ");

                let index = instruction.ip() as usize - address;
                for b in data[index..index + instruction.len()].iter() {
                    let s = format!("{:02x}", b);
                    #[allow(clippy::unnecessary_to_owned)]
                    output.buffer.push_str(&s.bright_white().to_string());
                    output.buffer.push(' ');
                }

                for _ in 0..8usize.saturating_sub(instruction.len()) {
                    output.buffer.push_str("   ");
                }

                formatter.format(&instruction, &mut output);
                output.buffer.push('\n');
            }
        } else {
            output
                .buffer
                .push_str(&format!("{:016x}\nno section", address));
        }
        output.buffer
    }

    pub(crate) fn disassemble_bytes(address: usize, data: &[u8]) -> String {
        disassemble_bytes_with_symbols(address, data, |_| None)
    }

    pub(crate) fn disassemble_bytes_with_symbols<F>(
        address: usize,
        data: &[u8],
        symbols: F,
    ) -> String
    where
        F: Fn(usize) -> Option<String>,
    {
        let mut output = Output::default();

        output.buffer.push_str(&format!(
            "{:016x} - {:016x}\n",
            address,
            address + data.len()
        ));

        output.buffer.push('\n');

        let mut formatter = IntelFormatter::new();
        formatter.options_mut().set_first_operand_char_index(8);
        for instruction in Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE) {
            let ip = format!("{:016x}", instruction.ip());
            output.buffer.push_str(&ip);
            output.buffer.push_str(":  ");

            let index = instruction.ip() as usize - address;
            for b in data[index..index + instruction.len()].iter() {
                let s = format!("{:02x}", b);
                #[allow(clippy::unnecessary_to_owned)]
                output.buffer.push_str(&s.bright_white().to_string());
                output.buffer.push(' ');
            }

            for _ in 0..8usize.saturating_sub(instruction.len()) {
                output.buffer.push_str("   ");
            }

            formatter.format(&instruction, &mut output);

            if instruction.op_kinds().any(|op| op == OpKind::NearBranch64) {
                if let Some(symbol) = symbols(instruction.near_branch64() as usize) {
                    output
                        .buffer
                        .push_str(&format!(" {}", symbol.bright_yellow().to_string()));
                }
            }
            output.buffer.push('\n');
        }
        output.buffer
    }

    pub(crate) fn get_xrefs(address: usize, data: &[u8]) -> Vec<(usize, usize)> {
        let mut xrefs = vec![];
        for instruction in Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE) {
            if instruction.op_kinds().any(|op| op == OpKind::NearBranch64) {
                xrefs.push((
                    instruction.ip() as usize,
                    instruction.near_branch64() as usize,
                ));
            }
        }
        xrefs
    }

    fn get_color(s: &str, kind: FormatterTextKind) -> ColoredString {
        match kind {
            FormatterTextKind::Directive | FormatterTextKind::Keyword => s.bright_yellow(),
            FormatterTextKind::Prefix | FormatterTextKind::Mnemonic => s.bright_red(),
            FormatterTextKind::Register => s.bright_blue(),
            FormatterTextKind::Number => s.bright_cyan(),
            _ => s.white(),
        }
    }
}

fn find_ext<P: AsRef<Path>>(dir: P, ext: &str) -> Result<Option<PathBuf>> {
    for f in fs::read_dir(dir)? {
        let f = f?.path();
        if f.is_file() && f.extension().and_then(std::ffi::OsStr::to_str) == Some(ext) {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

struct ScanResult<'a> {
    results: Vec<(&'a PatternConfig, Resolution)>,
}

fn scan_game<'patterns>(
    exe: &Executable,
    pattern_configs: &'patterns [PatternConfig],
) -> Result<ScanResult<'patterns>> {
    let mut results = vec![];

    struct PendingScan {
        index: usize,
        stages: ResolveStages,
        scan: Scan,
    }

    let mut scan_queue = pattern_configs
        .iter()
        .enumerate()
        .map(|(index, config)| PendingScan {
            index,
            stages: ResolveStages(vec![]),
            scan: config.scan.clone(), // TODO clone isn't ideal but makes handling multi-stage scans a lot easier
        })
        .collect::<Vec<_>>();

    while !scan_queue.is_empty() {
        let mut new_queue = vec![];
        for section in exe.object.sections() {
            let base_address = section.address() as usize;
            let section_name = section.name()?;
            let data = section.data()?;

            let pattern_scans = scan_queue
                .iter()
                .filter_map(|scan| {
                    scan.scan
                        .section
                        .map(|s| s == section.kind())
                        .unwrap_or(true)
                        .then(|| {
                            scan.scan
                                .scan_type
                                .get_pattern()
                                .map(|pattern| (scan, pattern))
                        })
                        .flatten()
                })
                .collect::<Vec<_>>();

            let xref_scans = scan_queue
                .iter()
                .filter_map(|scan| {
                    scan.scan
                        .section
                        .map(|s| s == section.kind())
                        .unwrap_or(true)
                        .then(|| scan.scan.scan_type.get_xref().map(|xref| (scan, xref)))
                        .flatten()
                })
                .collect::<Vec<_>>();

            let scan_results =
                patternsleuth::scanner::scan_memchr_lookup(&pattern_scans, base_address, data)
                    .into_iter()
                    .chain(patternsleuth::scanner::scan_xref_binary(
                        &xref_scans,
                        base_address,
                        data,
                    ));

            for (scan, address) in scan_results {
                let mut stages = scan.stages.clone();
                let action = (scan.scan.resolve)(
                    ResolveContext {
                        exe,
                        memory: &exe.memory,
                        section: section_name.to_owned(),
                        match_address: address,
                        pattern: pattern_scans[scan.index].1,
                    },
                    &mut stages,
                );
                match action {
                    ResolutionAction::Continue(new_scan) => {
                        new_queue.push(PendingScan {
                            index: scan.index,
                            stages,
                            scan: new_scan,
                        });
                    }
                    ResolutionAction::Finish(res) => {
                        results.push((
                            &pattern_configs[scan.index],
                            Resolution {
                                stages: stages.0,
                                res,
                            },
                        ));
                    }
                }
            }
        }
        scan_queue = new_queue;
    }

    Ok(ScanResult { results })
}

fn main() -> Result<()> {
    match Commands::parse() {
        Commands::Scan(command) => scan(command),
        Commands::Symbols(command) => symbols(command),
        Commands::BuildIndex(command) => index::build(command),
        Commands::ReadIndex(command) => index::read(command),
        Commands::SearchIndex(command) => index::search(command),
        Commands::ListIndex(command) => index::list(command),
        Commands::ViewSymbol(command) => index::view(command),
        Commands::BruteForce(command) => index::brute_force(command),
    }
}

fn scan(command: CommandScan) -> Result<()> {
    let patterns = if command.patterns.is_empty() && command.xref.is_empty() {
        let sig_filter = command.signature.into_iter().collect::<HashSet<_>>();
        get_patterns()?
            .into_iter()
            .filter(|p| {
                sig_filter
                    .is_empty()
                    .then_some(true)
                    .unwrap_or_else(|| sig_filter.contains(&p.sig))
            })
            .collect_vec()
    } else {
        command
            .patterns
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                PatternConfig::new(
                    Sig::Custom("arg".to_string()),
                    format!("pattern {i}"),
                    None,
                    p,
                    resolve_self,
                )
            })
            .chain(command.xref.into_iter().enumerate().map(|(i, p)| {
                PatternConfig::xref(
                    Sig::Custom("arg".to_string()),
                    format!("xref {i}"),
                    None,
                    p,
                    resolve_self,
                )
            }))
            .collect_vec()
    };
    let sigs = patterns
        .iter()
        .map(|p| p.sig.clone())
        .collect::<HashSet<_>>();

    let mut games: HashSet<String> = Default::default();

    let mut all: HashMap<(String, (&Sig, &String)), Vec<Resolution>> = HashMap::new();

    use colored::Colorize;
    use itertools::join;
    use prettytable::{format, row, Cell, Row, Table};

    for GameEntry { name, exe_path } in get_games(command.game)? {
        println!("{:?} {:?}", name, exe_path.display());
        let bin_data = fs::read(&exe_path)?;
        let exe = match Executable::read(
            &bin_data,
            &exe_path,
            command.symbols,
            !command.skip_exceptions,
        ) {
            Ok(exe) => exe,
            Err(err) => {
                println!("err reading {}: {}", exe_path.display(), err);
                continue;
            }
        };

        games.insert(name.to_string());

        let scan = scan_game(&exe, &patterns)?;

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
                        match &m.1.res {
                            ResolutionType::Address(address) => {
                                cells.push(Cell::new(&format!(
                                    "{}\n{}",
                                    m.0.name,
                                    disassemble::disassemble(
                                        &exe,
                                        *address,
                                        m.1.stages
                                            .is_empty()
                                            .then_some(m.0.scan.scan_type.get_pattern())
                                            .flatten()
                                    )
                                )));
                            }
                            ResolutionType::String(string) => {
                                cells.push(Cell::new(&format!("{:?}\n{:?}", m.0.name, string)));
                            }
                            ResolutionType::Count => {
                                #[allow(clippy::unnecessary_to_owned)]
                                cells.push(Cell::new(&format!("{}\ncount", m.0.name)));
                            }
                            ResolutionType::Failed => {
                                #[allow(clippy::unnecessary_to_owned)]
                                cells.push(Cell::new(&format!("{}\n{}", m.0.name, "failed".red())));
                            }
                        }
                        for (i, stage) in m.1.stages.iter().enumerate().rev() {
                            cells.push(Cell::new(&format!(
                                "stage[{}]\n{}",
                                i,
                                disassemble::disassemble(
                                    &exe,
                                    *stage,
                                    (i == 0)
                                        .then_some(m.0.scan.scan_type.get_pattern())
                                        .flatten()
                                )
                            )));
                        }
                        table.add_row(Row::new(cells));
                    }
                    cells.push(Cell::new(&table.to_string()));
                } else {
                    cells.push(Cell::new({
                        let mut lines = sig_scans
                            .iter()
                            // group and count matches by (pattern name, address)
                            .fold(
                                HashMap::<(&String, &ResolutionType), usize>::new(),
                                |mut map, m| {
                                    *map.entry((&m.0.name, &m.1.res)).or_default() += 1;
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

                                match &m.1 {
                                    ResolutionType::Address(address) => (
                                        format!("{:016x} {:?}{}", address, m.0, count)
                                            .normal()
                                            .to_string(),
                                        exe.symbols
                                            .as_ref()
                                            .and_then(|symbols| symbols.get(address)),
                                    ),
                                    ResolutionType::String(string) => (
                                        format!("{:?} {:?}{}", string, m.0, count)
                                            .normal()
                                            .to_string(),
                                        None,
                                    ),

                                    ResolutionType::Count => (
                                        format!("count {:?}{}", m.0, count).normal().to_string(),
                                        None,
                                    ),
                                    ResolutionType::Failed => (
                                        format!("failed {:?}{}", m.0, count).red().to_string(),
                                        None,
                                    ),
                                }
                            })
                            .collect::<Vec<_>>();
                        let max_len = lines.iter().map(|(line, _)| line.len()).max();
                        for (line, symbol) in &mut lines {
                            if let Some(symbol) = symbol {
                                line.push_str(&format!(
                                    "{}{}",
                                    " ".repeat(1 + max_len.unwrap() - line.len()),
                                    symbol.bright_yellow()
                                ));
                            }
                        }
                        &join(lines.iter().map(|(line, _)| line), "\n").to_string()
                    }));
                }
            } else {
                #[allow(clippy::unnecessary_to_owned)]
                cells.push(Cell::new(&"not found".red().to_string()));
            }

            table.add_row(Row::new(cells));
        }
        table.printstd();

        // fold current game scans into summary scans
        scan.results.into_iter().fold(&mut all, |map, m| {
            map.entry((name.to_string(), (&m.0.sig, &m.0.name)))
                .or_default()
                .push(m.1);
            map
        });

        println!();
    }

    if command.summary {
        #[derive(Debug, Default)]
        struct Summary {
            matches: usize,
            resolved: usize,
            failed: usize,
        }
        impl Summary {
            fn format(&self) -> String {
                if self.matches == 0 && self.failed == 0 && self.resolved == 0 {
                    "none".to_owned()
                } else {
                    format!("M={} R={} F={}", self.matches, self.resolved, self.failed)
                }
            }
        }

        let mut summary = Table::new();
        let title_strs: Vec<String> = ["".to_owned()]
            .into_iter()
            .chain(
                patterns
                    .iter()
                    .map(|conf| format!("{:?}({})", conf.sig, conf.name)),
            )
            .collect();
        summary.set_titles(Row::new(title_strs.iter().map(|s| Cell::new(s)).collect()));
        let mut totals = patterns.iter().map(|_| Summary::default()).collect_vec();

        for game in games.iter().sorted() {
            let mut row = vec![Cell::new(game)];

            let summaries: Vec<Summary> = patterns
                .iter()
                .map(|conf| {
                    let res = all.get(&(game.to_string(), (&conf.sig, &conf.name)));
                    if let Some(res) = res {
                        Summary {
                            matches: res.len(),
                            resolved: res
                                .iter()
                                .filter(|res| !matches!(res.res, ResolutionType::Failed))
                                .count(),
                            failed: res
                                .iter()
                                .filter(|res| matches!(res.res, ResolutionType::Failed))
                                .count(),
                        }
                    } else {
                        Summary {
                            matches: 0,
                            resolved: 0,
                            failed: 0,
                        }
                    }
                })
                .collect();

            for (i, s) in summaries.iter().enumerate() {
                if s.matches > 0 {
                    totals[i].matches += 1;
                }
                if s.resolved > 0 {
                    totals[i].resolved += 1;
                }
                if s.failed > 0 {
                    totals[i].failed += 1;
                }
            }

            let cell_strs: Vec<String> = summaries.iter().map(Summary::format).collect();
            row.extend(cell_strs.iter().map(|s| Cell::new(s)));
            summary.add_row(Row::new(row));
        }

        let total_strs = [format!("{}", games.len())]
            .into_iter()
            .chain(totals.iter().map(Summary::format))
            .collect_vec();
        summary.add_row(Row::new(
            total_strs.iter().map(|s| Cell::new(s)).collect_vec(),
        ));

        //let games: HashSet<String> = all.keys().map(|(game, _)| game).cloned().collect();
        //println!("{:#?}", all);

        summary.printstd();
    }

    Ok(())
}

fn symbols(command: CommandSymbols) -> Result<()> {
    let re = &command.symbol;
    let filter = |name: &_| re.iter().any(|re| re.is_match(name));

    use prettytable::{Cell, Row, Table};

    let mut cells = vec![];

    for GameEntry { name, exe_path } in get_games(command.game)? {
        if !exe_path.with_extension("pdb").exists() {
            continue;
        }

        println!("{:?} {:?}", name, exe_path.display());
        let bin_data = fs::read(&exe_path)?;
        let exe = match Executable::read(&bin_data, &exe_path, true, true) {
            Ok(exe) => exe,
            Err(err) => {
                println!("err reading {}: {}", exe_path.display(), err);
                continue;
            }
        };

        for (address, name) in exe.symbols.as_ref().unwrap() {
            if filter(name) {
                if let Some(exception) = exe.get_function(*address) {
                    let full_range = exception.full_range();
                    println!(
                        "{:016x} {:016x} {:08x} {}",
                        address,
                        full_range.end,
                        full_range.len(),
                        name
                    );
                    if exception.range.start != *address {
                        println!("MISALIGNED EXCEPTION ENTRY FOR {}", name);
                    } else {
                        cells.push((
                            name.clone(),
                            disassemble::disassemble_range(&exe, full_range),
                        ));
                    }
                } else {
                    println!("{:016x} [NO EXCEPT] {}", address, name);
                }
            }
        }
    }

    let mut table = Table::new();
    table.set_titles(cells.iter().map(|c| c.0.clone()).collect());
    table.add_row(Row::new(
        cells.into_iter().map(|c| Cell::new(&c.1)).collect(),
    ));
    table.printstd();

    Ok(())
}

struct GameEntry {
    name: String,
    exe_path: PathBuf,
}

fn get_games(filter: impl AsRef<[String]>) -> Result<Vec<GameEntry>> {
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

    fs::read_dir("games")?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sorted_by_key(|e| e.file_name())
        .map(|entry| -> Result<Option<GameEntry>> {
            let dir_name = entry.file_name();
            let name = dir_name.to_string_lossy().to_string();
            if !games_filter
                .is_empty()
                .then_some(true)
                .unwrap_or_else(|| games_filter.iter().any(|g| g.is_match(&name)))
            {
                return Ok(None);
            }

            let Some(exe_path) = find_ext(entry.path(), "exe")
                .transpose()
                .or_else(|| find_ext(entry.path(), "elf").transpose())
                .transpose()?
            else {
                return Ok(None);
            };
            Ok(Some(GameEntry { name, exe_path }))
        })
        .filter_map(|r| r.transpose())
        .collect::<Result<Vec<GameEntry>>>()
}

mod index {
    use super::*;

    use std::{borrow::Cow, collections::BTreeSet};

    use bincode::{Decode, Encode};
    use patternsleuth::ScanType;
    use prettytable::{Cell, Row, Table};
    use rayon::prelude::*;
    use rusqlite::{Connection, OptionalExtension};

    #[derive(Debug, Encode, Decode, PartialEq, PartialOrd, Eq, Ord, Hash)]
    struct SymbolKey {
        name: String,
    }

    #[derive(Debug, Encode, Decode, PartialEq, PartialOrd, Eq, Ord, Default)]
    struct SymbolValue {
        functions: BTreeSet<FunctionKey>,
    }

    #[derive(Debug, Encode, Decode, PartialEq, PartialOrd, Eq, Ord)]
    struct FunctionKey {
        address: usize,
        executable: String,
    }

    #[derive(Debug, Encode, Decode)]
    struct FunctionValue<'b> {
        bytes: Cow<'b, [u8]>,
    }

    pub(crate) fn search(command: CommandSearchIndex) -> Result<()> {
        let config = bincode::config::standard().with_big_endian();

        let db: sled::Db = sled::open("symbol_db")?;

        let functions_tree: sled::Tree = db.open_tree(b"functions")?;
        let symbols_tree: sled::Tree = db.open_tree(b"symbols")?;

        for key in symbols_tree.iter().keys() {
            let key_enc = key?;
            let key: SymbolKey = bincode::borrow_decode_from_slice(&key_enc, config)?.0;
            if key.name.contains(&command.symbol) {
                println!("{:X?}", key.name);

                let mut cells = vec![];
                if let Some(value_enc) = symbols_tree.get(key_enc)? {
                    let value: SymbolValue =
                        bincode::borrow_decode_from_slice(&value_enc, config)?.0;
                    for f in value.functions {
                        let fn_key_enc = bincode::encode_to_vec(&f, config)?;
                        if let Some(function_value) = functions_tree.get(fn_key_enc)? {
                            let function_value: FunctionValue =
                                bincode::borrow_decode_from_slice(&function_value, config)?.0;

                            cells.push((
                                f.executable,
                                disassemble::disassemble_bytes(f.address, &function_value.bytes),
                            ));
                        }
                    }
                }

                let mut table = Table::new();
                table.set_titles(cells.iter().map(|c| c.0.clone()).collect());
                table.add_row(Row::new(
                    cells.into_iter().map(|c| Cell::new(&c.1)).collect(),
                ));
                table.printstd();
            }
        }

        Ok(())
    }

    pub(crate) fn view(command: CommandViewSymbol) -> Result<()> {
        let conn = Connection::open("data.db")?;

        struct SqlFunction {
            game: String,
            address: usize,
            data: Vec<u8>,
        }

        let mut stmt = conn.prepare("SELECT game, address, data FROM functions JOIN symbols USING(game, address) WHERE symbol = ?1")?;
        let rows = stmt.query_map((&command.symbol,), |row| {
            Ok(SqlFunction {
                game: row.get(0)?,
                address: row.get(1)?,
                data: row.get(2)?,
            })
        })?;

        fn count_unequal<T: PartialEq>(a: &[T], b: &[T]) -> usize {
            a.iter().zip(b).filter(|(a, b)| a != b).count() + a.len().abs_diff(b.len())
        }

        struct Function {
            index: usize,
            sql: SqlFunction,
        }

        let mut functions = vec![];

        for row in rows {
            let sql = row?;

            let index = functions.len();
            functions.push(Function { index, sql });
        }

        if !functions.is_empty() {
            if let Some(pattern) =
                build_common_pattern(functions.iter().map(|f| &f.sql.data).collect::<Vec<_>>())
            {
                println!("{}", pattern);
            }

            for function in &functions {
                println!(
                    "{:2} {:08X} {}",
                    function.index, function.sql.address, function.sql.game
                );
            }

            let mut table = Table::new();
            table.add_row(Row::new(
                [Cell::new("")]
                    .into_iter()
                    .chain(
                        functions
                            .iter()
                            .enumerate()
                            .map(|(i, _)| Cell::new(&i.to_string())),
                    )
                    .collect(),
            ));
            let max = 100;

            let mut distances = HashMap::new();
            for (
                a_i,
                Function {
                    sql: SqlFunction { data: a, .. },
                    ..
                },
            ) in functions.iter().enumerate()
            {
                let mut cells = vec![Cell::new(&a_i.to_string())];
                for (
                    b_i,
                    Function {
                        sql: SqlFunction { data: b, .. },
                        ..
                    },
                ) in functions.iter().enumerate()
                {
                    let distance = count_unequal(&a[..a.len().min(max)], &b[..b.len().min(max)]);
                    distances.insert((a_i, b_i), distance);
                    distances.insert((b_i, a_i), distance);
                    cells.push(Cell::new(&distance.to_string()));
                }
                table.add_row(Row::new(cells));
            }
            table.printstd();

            let groups = if let Some(last) = functions.pop() {
                let mut groups = vec![vec![last]];
                while let Some(b) = functions.pop() {
                    let (d, group) = groups
                        .iter_mut()
                        .map(|group| {
                            (
                                group
                                    .iter()
                                    .map(|a| distances.get(&(a.index, b.index)).unwrap())
                                    .max()
                                    .unwrap(),
                                group,
                            )
                        })
                        .min_by_key(|(d, _)| *d)
                        .unwrap();
                    if *d < 50 {
                        group.push(b);
                    } else {
                        groups.push(vec![b]);
                    }
                }
                groups
            } else {
                vec![]
            };

            let mut patterns = vec![];

            for group in &groups {
                let pattern = build_common_pattern(
                    group
                        .iter()
                        .map(|f| &f.sql.data[..f.sql.data.len().min(max)])
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                println!("{}", pattern);
                patterns.push(pattern);
                println!(
                    "{:#?}",
                    group
                        .iter()
                        .map(|f| &f.sql.game)
                        .sorted()
                        .collect::<Vec<_>>()
                );
            }

            println!("./run.sh scan --skip-exceptions --summary\\");
            for pattern in patterns {
                println!("  -p '{}' \\", pattern);
            }

            for group in &groups {
                let mut table = Table::new();
                table.set_titles(group.iter().map(|f| &f.sql.game).collect());
                table.add_row(Row::new(
                    group
                        .iter()
                        .map(|f| {
                            Cell::new(&disassemble::disassemble_bytes_with_symbols(
                                f.sql.address,
                                &f.sql.data,
                                |address| -> Option<String> {
                                    command.show_symbols.then(||
                                    conn
                                        .query_row_and_then(
                                            "SELECT symbol FROM symbols WHERE game = ?1 AND address = ?2",
                                            (&f.sql.game, address),
                                            |row| row.get(0).optional(),
                                        )
                                        .ok()
                                        .flatten()).flatten()
                                }
                            ))
                        })
                        .collect(),
                ));
                table.printstd();
            }

            /*
            let mut table = Table::new();
            table.set_titles(cells.iter().map(|c| c.0.clone()).collect());
            table.add_row(Row::new(
                cells.into_iter().map(|c| Cell::new(&c.1)).collect(),
            ));
            table.printstd();
            */
        } else {
            println!("not found");
        }

        Ok(())
    }

    pub(crate) fn list(command: CommandListIndex) -> Result<()> {
        let config = bincode::config::standard().with_big_endian();

        let db: sled::Db = sled::open("symbol_db")?;

        let symbols_tree: sled::Tree = db.open_tree(b"symbols")?;

        for key in symbols_tree.iter().keys() {
            let key_enc = key?;
            let key: SymbolKey = bincode::borrow_decode_from_slice(&key_enc, config)?.0;
            println!("{}", key.name);
        }

        Ok(())
    }

    pub(crate) fn read(_command: CommandReadIndex) -> Result<()> {
        let config = bincode::config::standard().with_big_endian();

        let db: sled::Db = sled::open("symbol_db")?;

        let functions_tree: sled::Tree = db.open_tree(b"functions")?;
        let symbols_tree: sled::Tree = db.open_tree(b"symbols")?;

        for entry in symbols_tree.iter() {
            let (key, value) = entry?;
            let (key, value) = (
                bincode::borrow_decode_from_slice::<SymbolKey, _>(&key, config)?,
                bincode::borrow_decode_from_slice::<SymbolValue, _>(&value, config)?,
            );
            println!("{:X?} {:X?}", key, value);
        }

        for entry in functions_tree.iter() {
            let (key, value) = entry?;
            let (key, _value) = (
                bincode::borrow_decode_from_slice::<FunctionKey, _>(&key, config)?,
                bincode::borrow_decode_from_slice::<FunctionValue, _>(&value, config)?,
            );
            println!("{:X?}", key);
        }

        Ok(())
    }

    pub(crate) fn build(command: CommandBuildIndex) -> Result<()> {
        use crossbeam::channel::bounded;

        #[derive(Debug)]
        enum Insert {
            Function((String, usize, Vec<u8>)),
            Symbol((String, usize, String)),
            Xref((String, usize, usize, usize)),
        }

        let mut conn = Connection::open("data.db")?;

        conn.pragma_update(None, "synchronous", "OFF")?;
        conn.pragma_update(None, "journal_mode", "MEMORY")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS functions (
                game    TEXT NOT NULL,
                address INTEGER NOT NULL,
                data    BLOB NOT NULL
            )",
            (),
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS symbols (
                game      TEXT NOT NULL,
                address   INTEGER NOT NULL,
                symbol    TEXT NOT NULL
            )",
            (),
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS xrefs (
                game      TEXT NOT NULL,
                address_function    INTEGER NOT NULL,
                address_instruction INTEGER NOT NULL,
                address_reference   INTEGER NOT NULL
            )",
            (),
        )?;

        let (tx, rx) = bounded::<Insert>(0);

        crossbeam::scope(|scope| -> Result<()> {
            scope.spawn(|_| -> Result<()> {
                let transction = conn.transaction()?;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        Insert::Symbol(i) => {
                            let r = transction.execute(
                                "INSERT INTO symbols (game, address, symbol) VALUES (?1, ?2, ?3)",
                                (&i.0, i.1, &i.2),
                            );
                            if let Err(e) = r {
                                panic!("{:?} {:?}", e, i);
                            }
                        }
                        Insert::Function(i) => {
                            let r = transction.execute(
                                "INSERT INTO functions (game, address, data) VALUES (?1, ?2, ?3)",
                                i.clone(),
                            );
                            if let Err(e) = r {
                                panic!("{:?} {:?}", e, i);
                            }
                        }
                        Insert::Xref(i) => {
                            let r = transction.execute(
                                "INSERT INTO xrefs (game, address_function, address_instruction, address_reference) VALUES (?1, ?2, ?3, ?4)",
                                i.clone(),
                            );
                            if let Err(e) = r {
                                panic!("{:?} {:?}", e, i);
                            }
                        }
                    }
                }
                transction.commit()?;
                Ok(())
            });

            let games_with_symbols = get_games(command.game)?
                .into_iter()
                .filter(|g| g.exe_path.with_extension("pdb").exists())
                .collect::<Vec<_>>();

            use indicatif::ParallelProgressIterator;
            use indicatif::ProgressIterator;

            let m = indicatif::MultiProgress::new();
            let sty = indicatif::ProgressStyle::with_template(
                "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
            )
            .unwrap()
            .progress_chars("##-");

            let pb = m.add(indicatif::ProgressBar::new(games_with_symbols.len() as u64));
            pb.set_style(sty.clone());

            games_with_symbols
                .par_iter()
                .progress_with(pb.clone())
                .try_for_each(|GameEntry { name, exe_path }| -> Result<()> {
                    pb.set_message("total");

                    let bin_data = fs::read(exe_path)?;
                    let exe = match Executable::read(
                        &bin_data, exe_path, true, // symbols
                        true, // exceptions
                    ) {
                        Ok(exe) => exe,
                        Err(err) => {
                            println!("err reading {}: {}", exe_path.display(), err);
                            return Ok(());
                        }
                    };

                    let symbols = exe.symbols.as_ref().unwrap();

                    let pb = m.add(indicatif::ProgressBar::new(symbols.len() as u64));
                    pb.set_style(sty.clone());
                    pb.set_message(format!("inserting symbols for {}", name));

                    symbols.iter().progress_with(pb).try_for_each(
                        |(address, name)| -> Result<()> {
                            tx.send(Insert::Symbol((
                                exe_path.to_string_lossy().to_string(),
                                *address,
                                name.to_string(),
                            )))
                            .unwrap();

                            Ok(())
                        },
                    )?;

                    if let Some(functions) = exe.functions {
                        let pb = m.add(indicatif::ProgressBar::new(functions.len() as u64));
                        pb.set_style(sty.clone());
                        pb.set_message(format!("inserting functions for {}", name));

                        functions.iter().progress_with(pb).try_for_each(
                            |function| -> Result<()> {
                                let range = function.full_range();
                                let bytes = &exe.memory[range.clone()];

                                tx.send(Insert::Function((
                                    exe_path.to_string_lossy().to_string(),
                                    range.start,
                                    bytes.into(),
                                )))
                                .unwrap();

                                for (inst, xref) in disassemble::get_xrefs(range.start, bytes) {
                                    tx.send(Insert::Xref((
                                        exe_path.to_string_lossy().to_string(),
                                        range.start,
                                        inst,
                                        xref,
                                    )))
                                    .unwrap();
                                }

                                Ok(())
                            },
                        )?;
                    }
                    Ok(())
                })?;
            drop(tx);
            Ok(())
        })
        .unwrap()?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS functions_game_address_idx ON functions (game, address)",
            (),
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS symbols_game_address_idx ON symbols (game, address)",
            (),
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS symbols_symbol_idx ON symbols (symbol)",
            (),
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS xrefs_game_address_reference_idx ON xrefs (game, address_reference)",
            (),
        )?;

        Ok(())
    }

    pub(crate) fn brute_force(_command: CommandBruteForce) -> Result<()> {
        let config = bincode::config::standard().with_big_endian();

        let db: sled::Db = sled::open("symbol_db")?;

        let functions_tree: sled::Tree = db.open_tree(b"functions")?;
        let symbols_tree: sled::Tree = db.open_tree(b"symbols")?;

        let games = get_games([])?;

        let mut patterns = vec![];

        let date = chrono::Local::now();

        let mut log = csv::Writer::from_path(
            Path::new("scans").join(format!("scan-{}.log", date.format("%Y-%m-%d_%H-%M-%S"))),
        )?;

        for key in symbols_tree.iter().keys() {
            let key_enc = key?;
            let key: SymbolKey = bincode::borrow_decode_from_slice(&key_enc, config)?.0;

            if let Some(symbol_value) = symbols_tree.get(key_enc)? {
                let value: SymbolValue =
                    bincode::borrow_decode_from_slice(&symbol_value, config)?.0;

                let mut cells = vec![];
                let mut function_bodies = vec![];

                for f in value.functions {
                    let fn_key_enc = bincode::encode_to_vec(&f, config)?;
                    if let Some(function_value) = functions_tree.get(fn_key_enc)? {
                        let function_value: FunctionValue =
                            bincode::borrow_decode_from_slice(&function_value, config)?.0;

                        cells.push((
                            f.executable,
                            disassemble::disassemble_bytes(f.address, &function_value.bytes),
                        ));

                        function_bodies.push(function_value.bytes.to_vec());
                    }
                }

                if function_bodies.len() > 1 {
                    if let Some(pattern) = build_common_pattern(function_bodies) {
                        //println!("{} {}", key.name, pattern);
                        if let Ok(pattern) = Pattern::new(&pattern) {
                            if 10 < pattern.mask.iter().filter(|m| **m != 0).count() {
                                patterns.push((key.name, pattern));
                            }
                        }
                    }
                }
            }

            if patterns.len() > 10000 {
                let mut patterns = patterns
                    .drain(..)
                    .map(|(symbol, pattern)| {
                        PatternConfig::new(
                            Sig::Custom(symbol.clone()),
                            symbol,
                            None,
                            pattern,
                            resolve_self,
                        )
                    })
                    .collect::<Vec<_>>();

                let mut games_matched: HashMap<_, usize> = Default::default();

                for GameEntry { name, exe_path } in &games {
                    println!("p = {} {:?} {:?}", patterns.len(), name, exe_path.display());
                    let bin_data = fs::read(exe_path)?;
                    let exe = match Executable::read(&bin_data, exe_path, false, false) {
                        Ok(exe) => exe,
                        Err(err) => {
                            println!("err reading {}: {}", exe_path.display(), err);
                            continue;
                        }
                    };

                    let scan = scan_game(&exe, &patterns)?;

                    // group results by Sig
                    let folded_scans = scan
                        .results
                        .iter()
                        .map(|(config, m)| (&config.sig, (config, m)))
                        .fold(HashMap::new(), |mut map: HashMap<_, Vec<_>>, (k, v)| {
                            map.entry(k).or_default().push(v);
                            map
                        });

                    println!("{name}");
                    for (sig, group) in &folded_scans {
                        let sig = match sig {
                            Sig::Custom(name) => name,
                            _ => unreachable!(),
                        };
                        println!("\t{} {sig:10}", group.len());
                    }
                    let counts: HashMap<Sig, _> = folded_scans
                        .iter()
                        .map(|(sig, group)| ((*sig).clone(), group.len()))
                        .collect();

                    patterns.retain(|p| counts.get(&p.sig).map(|c| *c <= 1).unwrap_or(true));

                    for config in &patterns {
                        if counts.contains_key(&config.sig) {
                            let symbol = match &config.sig {
                                Sig::Custom(name) => name,
                                _ => unreachable!(),
                            };
                            *games_matched.entry(symbol.clone()).or_default() += 1;
                        }
                    }
                }

                for config in patterns {
                    let symbol = match config.sig {
                        Sig::Custom(name) => name,
                        _ => unreachable!(),
                    };
                    log.write_record([
                        games_matched.get(&symbol).unwrap().to_string(),
                        symbol,
                        match config.scan.scan_type {
                            ScanType::Pattern(pattern) => pattern.to_string(),
                            _ => unreachable!(),
                        },
                    ])?;
                }
                log.flush()?;
            }
        }

        Ok(())
    }

    fn build_common_pattern<B: AsRef<[u8]>>(function_bodies: impl AsRef<[B]>) -> Option<String> {
        let function_bodies = function_bodies.as_ref();
        if let Some(len) = function_bodies.iter().map(|b| b.as_ref().len()).min() {
            let mut sig = vec![];
            let mut mask = vec![];

            let mut last_eq = 0;

            for i in 0..len {
                if function_bodies.iter().map(|b| b.as_ref()[i]).all_equal() {
                    sig.push(function_bodies[0].as_ref()[i]);
                    mask.push(0xff);
                    last_eq = i + 1;
                } else {
                    sig.push(0);
                    mask.push(0);
                }
            }
            sig.truncate(last_eq);
            mask.truncate(last_eq);

            Some(
                sig.iter()
                    .zip(mask)
                    .map(|(sig, mask)| match mask {
                        0xff => Cow::Owned(format!("{sig:02X?}")),
                        0 => "??".into(),
                        _ => unreachable!(),
                    })
                    .join(" "),
            )
        } else {
            None
        }
    }
}
