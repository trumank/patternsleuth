use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use itertools::Itertools;
use object::{Object, ObjectSection};
use patternsleuth::patterns::resolve_self;
use patternsleuth::Executable;

use patternsleuth::{
    patterns::{get_patterns, Sig},
    scanner::Pattern,
    PatternConfig, Resolution, ResolutionAction, ResolutionType, ResolveContext, ResolveStages,
    Scan,
};

#[derive(Parser)]
enum Commands {
    Scan(CommandScan),
    Symbols(CommandSymbols),
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

    /// A signature to scan for (can be specified multiple times). Scans for all signatures if omitted
    #[arg(short, long, group = "scan", value_parser(|s: &_| Pattern::new(s)))]
    patterns: Vec<Pattern>,

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

mod disassemble {
    use std::ops::Range;

    use super::*;
    use colored::{ColoredString, Colorize};
    use iced_x86::{
        Decoder, DecoderOptions, Formatter, FormatterOutput, FormatterTextKind, IntelFormatter,
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
            let data = &section.data[(address - context * max_inst).saturating_sub(section.address)
                ..(address + context * max_inst).saturating_sub(section.address)];

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

            let start_address = (address - context * max_inst) as u64;
            let mut decoder = Decoder::with_ip(64, data, start_address, DecoderOptions::NONE);

            let instructions = decoder.iter().collect::<Vec<_>>();
            let instructions = if let Some((middle, _)) = instructions
                .iter()
                .enumerate()
                .find(|(_, inst)| inst.ip() >= address as u64)
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
    }
}

fn scan(command: CommandScan) -> Result<()> {
    let games_filter = command
        .game
        .into_iter()
        .map(|g| {
            Ok(globset::GlobBuilder::new(&g)
                .case_insensitive(true)
                .build()?
                .compile_matcher())
        })
        .collect::<Result<Vec<_>>>()?;

    let patterns = if command.patterns.is_empty() {
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
                    format!("arg {i}"),
                    None,
                    p,
                    resolve_self,
                )
            })
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

    for entry in fs::read_dir("games")?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sorted_by_key(|e| e.file_name())
    {
        let dir_name = entry.file_name();
        let game = dir_name.to_string_lossy().to_string();
        if !games_filter
            .is_empty()
            .then_some(true)
            .unwrap_or_else(|| games_filter.iter().any(|g| g.is_match(&game)))
        {
            continue;
        }

        let Some(exe_path) = find_ext(entry.path(), "exe")? else {
            continue;
        };

        println!("{:?} {:?}", game, exe_path.display());
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

        games.insert(game.to_string());

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
            map.entry((game.to_string(), (&m.0.sig, &m.0.name)))
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
    let games_filter = command
        .game
        .into_iter()
        .map(|g| {
            Ok(globset::GlobBuilder::new(&g)
                .case_insensitive(true)
                .build()?
                .compile_matcher())
        })
        .collect::<Result<Vec<_>>>()?;

    let re = &command.symbol;
    let filter = |name: &_| re.iter().any(|re| re.is_match(name));

    let mut games: HashSet<String> = Default::default();

    use prettytable::{Cell, Row, Table};

    let mut cells = vec![];

    for entry in fs::read_dir("games")?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sorted_by_key(|e| e.file_name())
    {
        let dir_name = entry.file_name();
        let game = dir_name.to_string_lossy().to_string();
        if !games_filter
            .is_empty()
            .then_some(true)
            .unwrap_or_else(|| games_filter.iter().any(|g| g.is_match(&game)))
        {
            continue;
        }

        let Some(exe_path) = find_ext(entry.path(), "exe")? else {
            continue;
        };
        if !exe_path.with_extension("pdb").exists() {
            continue;
        }

        println!("{:?} {:?}", game, exe_path.display());
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
                            game.clone(),
                            disassemble::disassemble_range(&exe, full_range),
                        ));
                    }
                } else {
                    println!("{:016x} [NO EXCEPT] {}", address, name);
                }
            }
        }

        games.insert(game.to_string());
    }

    let mut table = Table::new();
    table.set_titles(cells.iter().map(|c| c.0.clone()).collect());
    table.add_row(Row::new(
        cells.into_iter().map(|c| Cell::new(&c.1)).collect(),
    ));
    table.printstd();

    Ok(())
}