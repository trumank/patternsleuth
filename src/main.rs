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
use patternsleuth::MountedPE;
use strum::IntoEnumIterator;

use patternsleuth::{
    patterns::{get_patterns, Sig},
    PatternConfig, Resolution,
};

#[derive(Parser)]
struct CommandScan {
    /// A game to scan (can be specified multiple times). Scans everything if omitted
    #[arg(short, long)]
    game: Vec<String>,

    /// A signature to scan for (can be specified multiple times). Scans for all signatures if omitted
    #[arg(short, long)]
    signature: Vec<Sig>,
}

struct Log {
    addresses: Addresses,
    exe_name: String,
    exe_size: usize,
}

struct Addresses {
    /// base address of of MainExe module
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

fn main() -> Result<(), Box<dyn Error>> {
    let cli = CommandScan::parse();

    let sig_filter = cli.signature.into_iter().collect::<HashSet<_>>();
    let games_filter = cli.game.into_iter().collect::<HashSet<_>>();

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
    use prettytable::{row, Cell, Row, Table};

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
            .unwrap_or_else(|| games_filter.contains(&game))
        {
            continue;
        }
        let log_path = entry.path().join("UE4SS.log");

        let log = match read_addresses_from_log(log_path)
            .with_context(|| format!("{}: read UE4SS.log", game))
        {
            Ok(log) => Some(log),
            Err(e) => {
                println!("    Error: {:?}", e);
                None
            }
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

        println!(
            "{} {} exe_base={:016x?}",
            game,
            exe_path.display(),
            exe_base,
        );

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
                        if let Some(s) = config.section {
                            s == section.kind()
                        } else {
                            true
                        }
                    })
                    .map(|(config, m)| {
                        (
                            *config,
                            (config.resolve)(&mount, section_name.to_owned(), m),
                        )
                    })
                    .collect(),
            });
        }

        // group results by Sig
        let folded_scans = scans
            .iter()
            .flat_map(|scan| scan.results.iter())
            .map(|(config, m)| (&config.sig, (&config.name, m)))
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

            table.add_row(row![
                sig,
                sig_log
                    .map(|a| format!("{:016x}", a))
                    .unwrap_or("not found".to_owned()),
                folded_scans
                    .get(&sig)
                    .map(|m| join(
                        m.iter()
                            // group and count matches by (pattern name, address)
                            .fold(
                                HashMap::<(&String, Option<usize>), usize>::new(),
                                |mut map, m| {
                                    *map.entry((m.0, m.1.address)).or_default() += 1;
                                    map
                                }
                            )
                            .iter()
                            // sort by pattern name, then match address
                            .sorted_by_key(|(&m, _)| m)
                            .map(|(m, count)| {
                                // add count indicator if more than 1
                                let count = if *count > 1 {
                                    format!(" (x{count})")
                                } else {
                                    "".to_string()
                                };

                                let s = format!(
                                    "{} {:?}{}",
                                    m.1.map_or("failed".to_string(), |a| format!("{:016x}", a)),
                                    m.0,
                                    count,
                                );
                                if m.1.is_none() {
                                    s.red() // match addresss is None (resolution failed)
                                } else if sig_log.is_none() {
                                    s.normal() // log is not present so unsure if correct
                                } else if m.1.unwrap() == sig_log.unwrap() {
                                    s.green() // address matches log
                                } else {
                                    s.red() // match found but does not match log
                                }
                            }),
                        "\n"
                    )
                    .normal())
                    .unwrap_or("not found".to_owned().red()),
            ]);
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
                        resolved: res.iter().filter(|res| res.address.is_some()).count(),
                        failed: res.iter().filter(|res| res.address.is_none()).count(),
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
