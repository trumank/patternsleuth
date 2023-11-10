use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
};

use anyhow::Result;
use itertools::Itertools;
use patternsleuth::{Image, scanner::Pattern};
use prettytable::{Cell, Row, Table};
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension};

use crate::{disassemble, get_games, CommandBuildIndex, CommandViewSymbol, GameFileEntry};

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
            Function {
                sql: SqlFunction { data: a, .. },
                ..
            },
        ) in functions.iter().enumerate()
        {
            //let mut cells = vec![Cell::new(&a_i.to_string())];
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
                function.index, function.sql.address, function.sql.game
            );
        }

        for group in &groups {
            if let Some(pattern) = build_common_pattern(
                group
                    .iter()
                    .map(|f| &f.sql.data[..f.sql.data.len().min(max)])
                    .collect::<Vec<_>>(),
            ) {
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
        }

        println!("./run.sh scan --skip-exceptions --summary \\");
        for pattern in &patterns {
            println!("  -p '{}' \\", pattern);
        }

        for (group, pattern) in groups.iter().zip(patterns) {
            let mut table = Table::new();
            table.set_titles(group.iter().map(|f| &f.sql.game).collect());
            table.add_row(Row::new(
                group
                    .iter()
                    .map(|f| {
                        Cell::new(&disassemble::disassemble_bytes_with_symbols(
                            f.sql.address,
                            &f.sql.data,
                            Some(&Pattern::new(&pattern).unwrap()),
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

    let existing_games = {
        let mut stmt = conn.prepare("SELECT DISTINCT game FROM functions")?;
        let result = stmt
            .query_map((), |row| {
                Ok(std::path::PathBuf::from(row.get::<_, String>(0)?))
            })?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        result
    };

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

                // collect root exceptions / functions
                let mut functions = exe.exception_children_cache.keys().collect::<HashSet<_>>();
                for e in exe.exception_children_cache.values() {
                    for c in e {
                        functions.remove(&c.range.start);
                    }
                }

                let pb = m.add(indicatif::ProgressBar::new(functions.len() as u64));
                pb.set_style(sty.clone());
                pb.set_message(format!("inserting functions for {}", name));

                functions.iter().progress_with(pb).try_for_each(
                    |function| -> Result<()> {
                        let fns = exe.get_child_functions(exe.get_function(**function).unwrap().range.start);
                        let min = fns.iter().map(|f| f.range.start).min().unwrap();
                        let max = fns.iter().map(|f| f.range.end).max().unwrap();
                        let range = min..max;

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
