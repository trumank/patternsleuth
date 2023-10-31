use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use indicatif::ProgressBar;
use itertools::Itertools;
use patricia_tree::StringPatriciaMap;
use patternsleuth::patterns::resolve_self;
use patternsleuth::Executable;

use patternsleuth::scanner::Xref;
use patternsleuth::{
    patterns::{get_patterns, Sig},
    scanner::Pattern,
    PatternConfig, Resolution, ResolutionType,
};

mod sift4;

#[derive(Parser)]
enum Commands {
    Scan(CommandScan),
    Symbols(CommandSymbols),
    BuildIndex(CommandBuildIndex),
    ViewSymbol(CommandViewSymbol),
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

    /// A game process ID to attach to and scan
    #[arg(long)]
    pid: Option<i32>,

    /// A signature to scan for (can be specified multiple times). Scans for all signatures if omitted
    #[arg(short, long)]
    signature: Vec<Sig>,

    /// Show disassembly context for each stage of every match (I recommend only using with
    /// aggressive filters)
    #[arg(short, long)]
    disassemble: bool,

    /// Show disassembly context for each matched address
    #[arg(short, long)]
    disassemble_merged: bool,

    /// A pattern to scan for (can be specified multiple times)
    #[arg(short, long, value_parser(|s: &_| Pattern::new(s)))]
    patterns: Vec<Pattern>,

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
    use patternsleuth::MemoryTrait;

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
                section.address(),
                section.address() + section.data().len(),
                section.name(),
            ));

            let (is_fn, data, start_address) = if let Some(f) = exe.get_function(address) {
                let range = f.full_range();
                output.buffer.push_str(&format!(
                    "{:016x} - {:016x} = function\n",
                    range.start, range.end
                ));
                if let Some(symbols) = &exe.symbols {
                    if let Some(symbol) = symbols.get(&range.start) {
                        #[allow(clippy::unnecessary_to_owned)]
                        output.buffer.push_str(&symbol.bright_yellow().to_string());
                        output.buffer.push_str(&"".normal().to_string());
                        output.buffer.push('\n');
                    }
                }
                let start_address = range.start as u64;
                let data = section.range(range);
                (true, data, start_address)
            } else {
                output.buffer.push_str("no function");

                let data = &section.data()[(address - context * max_inst)
                    .saturating_sub(section.address())
                    ..(address + context * max_inst).saturating_sub(section.address())];
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
            let data = &section.range(range);

            output.buffer.push_str(&format!(
                "{:016x}\n{:016x} - {:016x} = {}\n",
                address,
                section.address(),
                section.address() + section.data().len(),
                section.name(),
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
                    #[allow(clippy::unnecessary_to_owned)]
                    output
                        .buffer
                        .push_str(&format!(" {}", symbol.bright_yellow().to_owned()));
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

fn main() -> Result<()> {
    match Commands::parse() {
        Commands::Scan(command) => scan(command),
        Commands::Symbols(command) => symbols(command),
        Commands::BuildIndex(command) => index::build(command),
        Commands::ViewSymbol(command) => index::view(command),
    }
}

fn scan(command: CommandScan) -> Result<()> {
    let sig_filter = command.signature.into_iter().collect::<HashSet<_>>();
    let include_default = command.patterns.is_empty() && command.xref.is_empty();
    let patterns = get_patterns()?
        .into_iter()
        .filter(|p| {
            sig_filter
                .is_empty()
                .then_some(include_default)
                .unwrap_or_else(|| sig_filter.contains(&p.sig))
        })
        .chain(
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
                })),
        )
        .collect_vec();

    let sigs = patterns
        .iter()
        .map(|p| p.sig.clone())
        .collect::<HashSet<_>>();

    let mut games: HashSet<String> = Default::default();

    let mut all: HashMap<(String, (&Sig, &String)), Vec<Resolution>> = HashMap::new();

    use colored::Colorize;
    use indicatif::ProgressIterator;
    use itertools::join;
    use prettytable::{format, row, Cell, Row, Table};

    enum Output {
        None,
        Stdout,
        Progress(ProgressBar),
    }

    impl Output {
        fn println<M: AsRef<str>>(&self, msg: M) {
            match self {
                Output::None => {}
                Output::Stdout => println!("{}", msg.as_ref()),
                Output::Progress(progress) => progress.println(msg),
            }
        }
    }

    let mut games_vec = vec![];

    if let Some(pid) = command.pid {
        games_vec.push(GameEntry::Process(GameProcessEntry { pid }));
    } else {
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
        let mut bin_data = None;

        let (name, exe) = match game {
            GameEntry::File(GameFileEntry { name, exe_path }) => {
                output.println(format!("{:?} {:?}", name, exe_path.display()));

                bin_data = Some(fs::read(exe_path)?);

                (
                    Cow::Borrowed(name),
                    match Executable::read(
                        bin_data.as_ref().unwrap(),
                        exe_path,
                        command.symbols,
                        !command.skip_exceptions,
                    ) {
                        Ok(exe) => exe,
                        Err(err) => {
                            output.println(format!("err reading {}: {}", exe_path.display(), err));
                            continue;
                        }
                    },
                )
            }
            GameEntry::Process(GameProcessEntry { pid }) => {
                output.println(format!("PID={pid}"));

                (
                    Cow::Owned(format!("PID={pid}")),
                    patternsleuth::process::read_image_from_pid(*pid)?,
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
                } else if command.disassemble_merged {
                    cells.push(Cell::new({
                        let cells = sig_scans
                            .iter()
                            .fold(
                                HashMap::<&ResolutionType, HashMap<&str, usize>>::new(),
                                |mut map, m| {
                                    *map.entry(&m.1.res)
                                        .or_default()
                                        .entry(&m.0.name)
                                        .or_default() += 1;
                                    map
                                },
                            )
                            .iter()
                            // sort by pattern name, then match address
                            .sorted_by_key(|&data| data.0)
                            .map(|(m, counts)| match &m {
                                ResolutionType::Address(address) => {
                                    let dis = disassemble::disassemble(&exe, *address, None);

                                    let mut lines = vec![];
                                    for (name, count) in counts.iter().sorted_by_key(|e| e.0) {
                                        let count = if *count > 1 {
                                            format!(" (x{count})")
                                        } else {
                                            "".to_string()
                                        };

                                        lines.push(
                                            format!("{:?}{}", name, count).normal().to_string(),
                                        );
                                    }
                                    lines.push(dis);

                                    Cell::new(&join(lines, "\n"))
                                }
                                _ => todo!(),
                            })
                            .collect::<Vec<_>>();

                        let mut table = Table::new();
                        table.set_format(*format::consts::FORMAT_NO_BORDER);

                        table.add_row(Row::new(cells));

                        &table.to_string()
                    }));
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
        let title_strs: Vec<String> = ["".into(), "unqiue addresses".into()]
            .into_iter()
            .chain(
                patterns
                    .iter()
                    .map(|conf| format!("{:?}({})", conf.sig, conf.name)),
            )
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
                            if let ResolutionType::Address(addr) = res.res {
                                matched_addresses.insert(addr);
                            }
                        }
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
            summary.add_row(Row::new(row));
        }

        let total_strs = [
            format!("total={}", games.len()),
            format!("0={} 1={} >1={}", no_matches, one_match, gt_one_match),
        ]
        .into_iter()
        .chain(totals.iter().map(Summary::format))
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

fn symbols(command: CommandSymbols) -> Result<()> {
    let re = &command.symbol;
    let filter = |name: &_| re.iter().any(|re| re.is_match(name));

    use prettytable::{Cell, Row, Table};

    let mut cells = vec![];

    for GameFileEntry { name, exe_path } in get_games(command.game)? {
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
                    let full_range = exception.full_range(); // TODO this now shows only the first exception block and misses any chained exceptions that may be covering the function
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

    fs::read_dir("games")?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(|entry| -> Result<Option<(String, PathBuf)>> {
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

mod index {
    use super::*;

    use std::borrow::Cow;

    use prettytable::{Cell, Row, Table};
    use rayon::prelude::*;
    use rusqlite::{Connection, OptionalExtension};

    pub(crate) fn view(command: CommandViewSymbol) -> Result<()> {
        println!("{:?}", command.symbol);
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
            println!("{} total functions", functions.len());

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

            println!("./run.sh scan --skip-exceptions --summary \\");
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
                .try_for_each(|GameFileEntry { name, exe_path }| -> Result<()> {
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
