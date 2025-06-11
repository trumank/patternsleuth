use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
};

use anyhow::Result;
use itertools::Itertools;
use patternsleuth::{image::Image, scanner::Pattern, PatternConfig};
use prettytable::{Cell, Row, Table};
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension};

use crate::{
    disassemble, get_games, CommandAutoGen, CommandBuildIndex, CommandViewSymbol, GameFileEntry,
};

fn generate_patterns_for_symbol(symbol: &str) -> Result<Vec<Pattern>> {
    let conn = Connection::open("data.db")?;

    struct SqlFunction {
        data: Vec<u8>,
    }

    let mut stmt = conn.prepare(
        "SELECT data FROM functions JOIN symbols USING(game, address) WHERE symbol = ?1",
    )?;
    let rows = stmt.query_map((symbol,), |row| Ok(SqlFunction { data: row.get(0)? }))?;

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
    }

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

    let patterns = groups
        .iter()
        .flat_map(|g| {
            build_common_pattern(
                g.iter()
                    .map(|f| &f.sql.data[..f.sql.data.len().min(max)])
                    .collect::<Vec<_>>(),
            )
            .map(|s| Pattern::new(s).unwrap())
        })
        .collect::<Vec<_>>();

    Ok(patterns)
}

pub(crate) fn auto_gen(_command: CommandAutoGen) -> Result<()> {
    let conn = Connection::open("data.db")?;

    #[derive(Debug)]
    struct QueryResult {
        symbol: String,
    }

    let mut stmt = conn.prepare("SELECT COUNT(*) AS count, symbol FROM symbols JOIN functions USING(game, address) WHERE demangled LIKE '% %' GROUP BY symbol HAVING count > 20")?;
    let rows = stmt.query_map((), |row| {
        Ok(QueryResult {
            symbol: row.get(1)?,
        })
    })?;

    let mut pattern_map: HashMap<String, Vec<Pattern>> = Default::default();

    for row in rows {
        let row = row?;
        dbg!(&row);
        let patterns = generate_patterns_for_symbol(&row.symbol)?;
        pattern_map
            .entry(row.symbol)
            .or_default()
            .extend(patterns.into_iter());
    }
    println!("testing {} symbols", pattern_map.len());

    let mut scan_patterns = vec![];

    for (symbol, patterns) in &pattern_map {
        for (i, pattern) in patterns.iter().enumerate() {
            scan_patterns.push(PatternConfig::new(
                (i, symbol.as_str()),
                "".into(),
                None,
                pattern.clone(),
            ))
        }
    }

    let mut matches: HashMap<&str, usize> = Default::default();
    let mut bad = HashSet::new();

    let games_vec = get_games([])?;
    for GameFileEntry { name, exe_path } in games_vec {
        println!("{:?} {:?}", name, exe_path.display());

        let bin_data = fs::read(&exe_path)?;

        let exe = match Image::builder().build(&bin_data) {
            Ok(exe) => exe,
            Err(err) => {
                println!("err reading {}: {}", exe_path.display(), err);
                continue;
            }
        };

        let scan = exe.scan(&scan_patterns)?;

        // group results by Sig
        let folded_scans = scan
            .results
            .iter()
            .map(|(config, m)| (&config.sig.1, (config.sig.0, m.address)))
            .fold(
                HashMap::new(),
                |mut map: HashMap<_, HashMap<usize, Vec<_>>>, (k, (i, v))| {
                    map.entry(k).or_default().entry(i).or_default().push(v);
                    map
                },
            );

        let mut to_remove = HashSet::new();

        for (symbol, results) in folded_scans {
            let mut any_match = false;
            for (pattern_index, addresses) in results {
                if addresses.len() > 1 {
                    let sig = (pattern_index, *symbol);
                    println!("\t{sig:?} matched multiple, removing");
                    to_remove.insert(sig);
                    bad.insert(sig);
                } else {
                    println!("\t{:?}: {addresses:x?}", (pattern_index, symbol));
                    any_match = true;
                }
            }
            if any_match {
                *matches.entry(symbol).or_default() += 1;
            }
        }
        drop(scan);
        scan_patterns.retain(|p| !to_remove.contains(&p.sig));
    }

    let mut output: HashMap<_, Vec<_>> = Default::default();

    for (symbol, count) in matches.iter().sorted_by_key(|(_, v)| *v) {
        println!("{count}: {symbol}");
        for (index, pattern) in pattern_map.get(*symbol).unwrap().iter().enumerate() {
            if !bad.contains(&(index, *symbol)) {
                println!("\t{pattern}");
                output.entry(symbol).or_default().push(format!("{pattern}"));
            }
        }
    }

    std::fs::write("patterns.json", serde_json::to_string(&output)?)?;

    Ok(())
}

pub(crate) fn view(command: CommandViewSymbol) -> Result<()> {
    println!("symbols={:?}", command.symbol);
    let conn = Connection::open("data.db")?;

    struct Function {
        game: String,
        address: usize,
        data: Vec<u8>,
    }

    struct IndexedFunction {
        index: usize,
        function: Function,
    }

    let mut functions = vec![];
    for symbol in command.symbol {
        let mut stmt = conn.prepare("SELECT game, address, data FROM functions JOIN symbols USING(game, address) WHERE symbol = ?1")?;
        for row in stmt.query_map((&symbol,), |row| {
            Ok(Function {
                game: row.get(0)?,
                address: row.get(1)?,
                data: row.get(2)?,
            })
        })? {
            functions.push(row?)
        }
    }

    for function in command.function {
        let data = fs::read(&function.path)?;
        let img = Image::builder().build(&data).unwrap();
        functions.push(Function {
            game: function.path,
            address: function.start,
            data: img.memory[function.start..function.end].to_vec(),
        });
    }

    let resolvers = command
        .resolver
        .into_iter()
        .map(|res| res.getter)
        .collect::<Vec<_>>();

    if !resolvers.is_empty() {
        let mut games: HashSet<String> = Default::default();

        for game in crate::get_games([])? {
            #[allow(unused_assignments)]
            let mut bin_data = None;

            let GameFileEntry { name, exe_path } = game;

            bin_data = Some(fs::read(&exe_path)?);

            let exe = {
                let bin_data = bin_data.as_ref().unwrap();
                match Image::builder().functions(false).build(bin_data) {
                    Ok(exe) => exe,
                    Err(err) => {
                        println!("err reading {}: {err}", exe_path.display());
                        continue;
                    }
                }
            };

            games.insert(name.to_string());

            let resolution = exe.resolve_many(&resolvers);
            println!("{resolution:#x?}");
            for res in resolution.into_iter().flatten() {
                let start = res.get().unwrap();
                let bounds = patternsleuth::disassemble::function_range(&exe, start)?;
                functions.push(Function {
                    game: exe_path.to_string_lossy().to_string(),
                    address: start,
                    data: exe.memory[bounds].to_vec(),
                });
            }
        }
    }

    let mut functions = functions
        .into_iter()
        .enumerate()
        .map(|(index, function)| IndexedFunction { index, function })
        .collect::<Vec<_>>();

    fn count_unequal<T: PartialEq>(a: &[T], b: &[T]) -> usize {
        a.iter().zip(b).filter(|(a, b)| a != b).count() + a.len().abs_diff(b.len())
    }

    if !functions.is_empty() {
        /*
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
        */
        let max = 100;

        let mut distances = HashMap::new();
        for (
            a_i,
            IndexedFunction {
                function: Function { data: a, .. },
                ..
            },
        ) in functions.iter().enumerate()
        {
            //let mut cells = vec![Cell::new(&a_i.to_string())];
            for (
                b_i,
                IndexedFunction {
                    function: Function { data: b, .. },
                    ..
                },
            ) in functions.iter().enumerate()
            {
                let distance = count_unequal(&a[..a.len().min(max)], &b[..b.len().min(max)]);
                distances.insert((a_i, b_i), distance);
                distances.insert((b_i, a_i), distance);
                //cells.push(Cell::new(&distance.to_string()));
            }
            //table.add_row(Row::new(cells));
        }
        //table.printstd();

        let function_count = functions.len();

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

        println!(
            "{} total functions in {} group",
            function_count,
            groups.len()
        );

        for function in &functions {
            println!(
                "{:2} {:08X} {}",
                function.index, function.function.address, function.function.game
            );
        }

        for group in &groups {
            if let Some(pattern) = build_common_pattern(
                group
                    .iter()
                    .map(|f| &f.function.data[..f.function.data.len().min(max)])
                    .collect::<Vec<_>>(),
            ) {
                println!("{pattern}");
                patterns.push(pattern);
                println!(
                    "{:#?}",
                    group
                        .iter()
                        .map(|f| &f.function.game)
                        .sorted()
                        .collect::<Vec<_>>()
                );
            }
        }

        println!("./run.sh scan --skip-exceptions --summary \\");
        for pattern in &patterns {
            println!("  -p '{pattern}' \\");
        }

        for (group, pattern) in groups.iter().zip(patterns) {
            let mut table = Table::new();
            table.set_titles(group.iter().map(|f| &f.function.game).collect());
            table.add_row(Row::new(
                group
                    .iter()
                    .map(|f| {
                        Cell::new(&disassemble::disassemble_bytes_with_symbols(
                            f.function.address,
                            &f.function.data,
                            Some(&Pattern::new(&pattern).unwrap()),
                            |address| -> Option<String> {
                                command.show_symbols.then(||
                                conn
                                    .query_row_and_then(
                                        "SELECT symbol FROM symbols WHERE game = ?1 AND address = ?2",
                                        (&f.function.game, address),
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
        Symbol {
            game: String,
            address: usize,
            symbol: String,
            demangled: String,
        },
        Xref((String, usize, usize, usize)),
    }

    let mut conn = Connection::open("data.db")?;

    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "cache_size", "1000000")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;

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
            symbol    TEXT NOT NULL,
            demangled TEXT NOT NULL
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

    let existing_games = {
        let mut stmt = conn.prepare("SELECT DISTINCT game FROM functions")?;
        #[warn(clippy::let_and_return)]
        let result = stmt
            .query_map((), |row| {
                Ok(std::path::PathBuf::from(row.get::<_, String>(0)?))
            })?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        result
    };

    crossbeam::scope(|scope| -> Result<()> {
        scope.spawn(|_| -> Result<()> {
            let transction = conn.transaction()?;
            while let Ok(msg) = rx.recv() {
                match msg {
                    Insert::Symbol{game, address, symbol, demangled} => {
                        let r = transction.execute(
                            "INSERT INTO symbols (game, address, symbol, demangled) VALUES (?1, ?2, ?3, ?4)",
                            (game, address, symbol, demangled),
                        );
                        if let Err(e) = r {
                            panic!("{e:?}");
                        }
                    }
                    Insert::Function(i) => {
                        let r = transction.execute(
                            "INSERT INTO functions (game, address, data) VALUES (?1, ?2, ?3)",
                            i.clone(),
                        );
                        if let Err(e) = r {
                            panic!("{e:?} {i:?}");
                        }
                    }
                    Insert::Xref(i) => {
                        let r = transction.execute(
                            "INSERT INTO xrefs (game, address_function, address_instruction, address_reference) VALUES (?1, ?2, ?3, ?4)",
                            i.clone(),
                        );
                        if let Err(e) = r {
                            panic!("{e:?} {i:?}");
                        }
                    }
                }
            }
            transction.commit()?;
            Ok(())
        });

        let games_with_symbols = get_games(command.game)?
            .into_iter()
            .filter(|g| !existing_games.contains(&g.exe_path) && g.exe_path.with_extension("pdb").exists())
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
                let exe = match Image::builder()
                    .functions(true)
                    .symbols(exe_path)
                    .build(&bin_data)
                {
                    Ok(exe) => exe,
                    Err(err) => {
                        println!("err reading {}: {}", exe_path.display(), err);
                        return Ok(());
                    }
                };

                let symbols = exe.symbols.as_ref().unwrap();

                let pb = m.add(indicatif::ProgressBar::new(symbols.len() as u64));
                pb.set_style(sty.clone());
                pb.set_message(format!("inserting symbols for {name}"));

                symbols.iter().progress_with(pb).try_for_each(
                    |(address, sym)| -> Result<()> {
                        tx.send(Insert::Symbol{
                            game: exe_path.to_string_lossy().to_string(),
                            address: *address,
                            symbol: sym.name.to_string(),
                            demangled: sym.demangle(),
                        })
                        .unwrap();

                        Ok(())
                    },
                )?;

                // collect root exceptions / functions
                let functions = exe.get_root_functions()?;

                let pb = m.add(indicatif::ProgressBar::new(functions.len() as u64));
                pb.set_style(sty.clone());
                pb.set_message(format!("inserting functions for {name}"));

                functions.iter().progress_with(pb).try_for_each(
                    |function| -> Result<()> {
                        let range = function;

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
        "CREATE INDEX IF NOT EXISTS symbols_symbol_game_address_idx ON symbols (symbol, game, address)",
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
            } else if i == 0 {
                // first byte cannot be wildcard
                return None;
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
