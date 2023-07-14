use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use clap::Parser;
use itertools::Itertools;
use object::{Object, ObjectSection};
use patternsleuth::{MountedPE, ResolutionType, ResolveContext};
use strum::IntoEnumIterator;

use patternsleuth::{
    patterns::{get_patterns, Sig},
    PatternConfig, Resolution,
};

#[derive(clap::Subcommand)]
enum Action {
    Functions(ActionFunctions),
    Sym(ActionSym),
}

#[derive(Parser)]
pub struct ActionFunctions {
    exe: std::path::PathBuf,
    other_exe: std::path::PathBuf,
}

#[derive(Parser)]
pub struct ActionSym {
    exe: std::path::PathBuf,
    other_exe: std::path::PathBuf,
    address: Option<String>,
}

#[derive(Parser)]
struct CommandScan {
    #[command(subcommand)]
    action: Option<Action>,

    /// A game to scan (can be specified multiple times). Scans everything if omitted. Supports
    /// globs
    #[arg(short, long)]
    game: Vec<String>,

    /// A signature to scan for (can be specified multiple times). Scans for all signatures if omitted
    #[arg(short, long)]
    signature: Vec<Sig>,

    /// Show disassembly context for each stage of every match (I recommend only using with
    /// aggressive filters)
    #[arg(short, long)]
    disassemble: bool,
}

struct Log {
    addresses: Addresses,
    exe_name: String,
    exe_size: usize,
}

struct Addresses {
    /// base address of of MainExe module
    #[allow(dead_code)]
    main_exe: usize,
    /// addresses of Sigs relative to MainExe
    addresses: HashMap<Sig, usize>,
}

fn read_addresses_from_log<P: AsRef<Path>>(path: P) -> Result<Log> {
    let mut addresses = HashMap::new();

    let re_exe_path =
        regex::Regex::new(r"game executable: .+[\\/](.+\.exe) \(([0-9]+) bytes\)$").unwrap();
    let mut exe_path = None;

    let re_main_exe = regex::Regex::new(r"MainExe @ 0x([0-9a-f]+) size=0x([0-9a-f]+)").unwrap();
    let mut main_exe = None;

    let re_address = regex::Regex::new(r"([^ ]+) address: 0x([0-9a-f]+)").unwrap();
    for line in BufReader::new(fs::File::open(path)?).lines() {
        let line = line?;
        if let Some(captures) = re_address.captures(&line) {
            if let Ok(sig) = Sig::from_str(&captures[1]) {
                let address = usize::from_str_radix(&captures[2], 16)?;
                if addresses.get(&sig).map(|a| *a != address).unwrap_or(false) {
                    bail!("found multiple unique addresses for \"{}\"", sig);
                }
                addresses.insert(sig, address);
            }
        } else if let Some(captures) = re_main_exe.captures(&line) {
            main_exe = Some(usize::from_str_radix(&captures[1], 16)?);
        } else if let Some(captures) = re_exe_path.captures(&line) {
            exe_path = Some((captures[1].to_owned(), usize::from_str(&captures[2])?));
        }
    }
    let (exe_name, exe_size) = exe_path.context("game executable path not found in log")?;
    let main_exe = main_exe.context("MainExe module not found in log")?;

    // compute addresses relative to base module
    let addresses = addresses
        .into_iter()
        .map(|(k, v)| (k, v - main_exe))
        .collect::<HashMap<_, _>>();
    Ok(Log {
        exe_name,
        exe_size,
        addresses: Addresses {
            main_exe,
            addresses,
        },
    })
}

mod disassemble {
    use colored::{ColoredString, Colorize};
    use iced_x86::{
        Decoder, DecoderOptions, Formatter, FormatterOutput, FormatterTextKind, IntelFormatter,
    };
    use patternsleuth::{MountedPE, Pattern};

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
        memory: &MountedPE,
        address: usize,
        pattern: Option<&Pattern>,
    ) -> String {
        let context = 20; // number of instructions before and after
        let max_inst = 16; // max size of x86 instruction in bytes

        let mut output = Output::default();

        if let Some(section) = memory.get_section_containing(address) {
            let data = &section.data[(address - context * max_inst).saturating_sub(section.address)
                ..(address + context * max_inst).saturating_sub(section.address)];

            output.buffer.push_str(&format!(
                "{:016x}\n{}\n{:016x} - {:016x}\n\n",
                address,
                section.name,
                section.address,
                section.address + section.data.len()
            ));

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
                            let offset = (instruction.ip() as usize)
                                .checked_sub(address)?
                                .checked_add(i)?;
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

    pub(crate) fn disassemble_fixed(data: &[u8], address: usize) -> String {
        let mut output = Output::default();

        let decoder = Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE);

        let mut formatter = IntelFormatter::new();
        formatter.options_mut().set_first_operand_char_index(8);
        for instruction in decoder {
            let ip = format!("{:016x}", instruction.ip());
            output.buffer.push_str(&ip);
            output.buffer.push_str(":  ");

            let index = (instruction.ip() - address as u64) as usize;
            for b in &data[index..index + instruction.len()] {
                #[allow(clippy::unnecessary_to_owned)]
                output
                    .buffer
                    .push_str(&format!("{:02x} ", b).bright_black().to_string());
            }

            for _ in 0..8usize.saturating_sub(instruction.len()) {
                output.buffer.push_str("   ");
            }

            formatter.format(&instruction, &mut output);

            if instruction.is_ip_rel_memory_operand() {
                output.buffer.push_str(" rel");
            }
            output
                .buffer
                .push_str(&format!(" {:x}", instruction.near_branch_target()));

            output.buffer.push('\n');
        }
        output.buffer
    }

    pub(crate) fn disassemble_fixed_small(data: &[u8], address: usize) -> String {
        let mut output = String::new();

        let decoder = Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE);

        let mut formatter = IntelFormatter::new();
        formatter.options_mut().set_first_operand_char_index(8);
        for instruction in decoder {
            formatter.format(&instruction, &mut output);
            output.push('\n');
        }
        output
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

fn main() -> Result<(), Box<dyn Error>> {
    let cli = CommandScan::parse();

    match cli.action {
        Some(Action::Functions(action)) => {
            return patternsleuth::diff::functions(action.exe, action.other_exe)
        }
        Some(Action::Sym(action)) => {
            return patternsleuth::diff::sym(action.exe, action.other_exe, action.address)
        }
        None => {}
    }

    let sig_filter = cli.signature.into_iter().collect::<HashSet<_>>();
    let games_filter = cli
        .game
        .into_iter()
        .map(|g| {
            Ok(globset::GlobBuilder::new(&g)
                .case_insensitive(true)
                .build()?
                .compile_matcher())
        })
        .collect::<Result<Vec<_>>>()?;

    let patterns = get_patterns()?
        .into_iter()
        .filter(|p| {
            sig_filter
                .is_empty()
                .then_some(true)
                .unwrap_or_else(|| sig_filter.contains(&p.sig))
        })
        .collect_vec();
    let pat = patterns
        .iter()
        .map(|config| (config, &config.pattern))
        .collect_vec();
    let pat_ref = pat.iter().map(|(id, p)| (id, *p)).collect_vec();

    let mut games: HashSet<String> = Default::default();

    let mut all: HashMap<(String, (&Sig, &String)), Vec<Resolution>> = HashMap::new();

    use colored::Colorize;
    use itertools::join;
    use prettytable::{format, row, Cell, Row, Table};

    'loop_games: for entry in fs::read_dir("games")?
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
        let log_path = entry.path().join("UE4SS.log");

        let log = if log_path.exists() {
            match read_addresses_from_log(log_path)
                .with_context(|| format!("{}: read UE4SS.log", game))
            {
                Ok(log) => Some(log),
                Err(e) => {
                    println!("Error: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        let exe_path = if let Some(ref log) = log {
            entry.path().join(&log.exe_name)
        } else {
            'exe: {
                for f in fs::read_dir(entry.path())? {
                    let f = f?.path();
                    if f.is_file() && f.extension().and_then(std::ffi::OsStr::to_str) == Some("exe")
                    {
                        break 'exe f;
                    }
                }
                continue 'loop_games;
            }
        };

        let bin_data = fs::read(&exe_path)
            .with_context(|| format!("reading game exe {}", exe_path.display()))?;
        if let Some(log) = &log {
            if log.exe_size != bin_data.len() {
                println!("size mismatch: log indicates {} bytes but {} is {} bytes. is this the correct exe?", log.exe_size, exe_path.display(), bin_data.len());
                continue 'loop_games;
            }
        }
        let obj_file = object::File::parse(&*bin_data)?;
        let exe_base = obj_file.relative_address_base() as usize;
        let mount = MountedPE::new(&obj_file)?;

        games.insert(game.to_string());

        println!("{:?} {:?}", game, exe_path.display());

        struct Scan<'a> {
            base_address: usize,
            results: Vec<(&'a PatternConfig, Resolution)>,
        }

        // perform scans for game
        let mut scans = vec![];
        for section in obj_file.sections() {
            let base_address = section.address() as usize;
            let section_name = section.name()?;
            let data = section.data()?;
            scans.push(Scan {
                base_address,
                results: patternsleuth::scanner::scan(pat_ref.as_slice(), base_address, data)
                    .into_iter()
                    .filter(|(config, _)| {
                        config.section.map(|s| s == section.kind()).unwrap_or(true)
                    })
                    .map(|(config, m)| {
                        (
                            *config,
                            (config.resolve)(ResolveContext {
                                memory: &mount,
                                section: section_name.to_owned(),
                                match_address: m,
                            }),
                        )
                    })
                    .collect(),
            });
        }

        // group results by Sig
        let folded_scans = scans
            .iter()
            .flat_map(|scan| scan.results.iter())
            .map(|(config, m)| (&config.sig, (config, m)))
            .fold(HashMap::new(), |mut map, (k, v)| {
                map.entry(k).or_insert_with(Vec::new).push(v);
                map
            });

        let mut table = Table::new();
        table.set_titles(row!["sig", "log", "offline scan"]);

        for sig in Sig::iter().filter(|sig| {
            sig_filter
                .is_empty()
                .then_some(true)
                .unwrap_or_else(|| sig_filter.contains(sig))
        }) {
            // get validated Sig addresses from log
            let sig_log = log
                .as_ref()
                .and_then(|a| a.addresses.addresses.get(&sig))
                .map(|a| a + exe_base);

            let mut cells = vec![];
            cells.push(Cell::new(&sig.to_string()));
            cells.push(Cell::new(
                &sig_log
                    .map(|a| format!("{:016x}", a))
                    .unwrap_or("not found".to_owned()),
            ));

            if let Some(sig_scans) = folded_scans.get(&sig) {
                if cli.disassemble {
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
                                        &mount,
                                        *address,
                                        m.1.stages.is_empty().then_some(&m.0.pattern)
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
                                    &mount,
                                    *stage,
                                    (i == 0).then_some(&m.0.pattern)
                                )
                            )));
                        }
                        table.add_row(Row::new(cells));
                    }
                    cells.push(Cell::new(&table.to_string()));
                } else {
                    cells.push(Cell::new(
                        &join(
                            sig_scans
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
                                        ResolutionType::Address(address) => {
                                            let s = format!("{:016x} {:?}{}", address, m.0, count);
                                            if let Some(sig_address) = sig_log {
                                                if *address == sig_address {
                                                    s.green() // address matches log
                                                } else {
                                                    s.red() // match found but does not match log
                                                }
                                            } else {
                                                s.normal() // log is not present so unsure if correct
                                            }
                                        }
                                        ResolutionType::String(string) => {
                                            format!("{:?} {:?}{}", string, m.0, count).normal()
                                        }

                                        ResolutionType::Count => {
                                            format!("count {:?}{}", m.0, count).normal()
                                        }
                                        ResolutionType::Failed => {
                                            format!("failed {:?}{}", m.0, count).red()
                                        }
                                    }
                                }),
                            "\n",
                        )
                        .to_string(),
                    ));
                }
            } else {
                #[allow(clippy::unnecessary_to_owned)]
                cells.push(Cell::new(&"not found".red().to_string()));
            }

            table.add_row(Row::new(cells));
        }
        table.printstd();

        // fold current game scans into summary scans
        scans
            .into_iter()
            .flat_map(|scan| scan.results.into_iter())
            .fold(&mut all, |map, m| {
                map.entry((game.to_string(), (&m.0.sig, &m.0.name)))
                    .or_default()
                    .push(m.1);
                map
            });

        println!();
    }

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

    Ok(())
}
