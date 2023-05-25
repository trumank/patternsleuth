use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use object::{Object, ObjectSection};
use strum::IntoEnumIterator;

use patternsleuth::*;

#[derive(
    Debug, Hash, Eq, PartialEq, PartialOrd, strum::Display, strum::EnumString, strum::EnumIter,
)]
enum Sig {
    #[strum(serialize = "FName::ToString")]
    FNameToString,
    #[strum(serialize = "FName::FName")]
    FNameFName,
    GMalloc,
    GUObjectArray,
    GNatives,
    //ProcessInternal, // not found by pattern scan
    //ProcessLocalScriptFunction, // not found by pattern scan
    #[strum(serialize = "StaticConstructObject_Internal")]
    StaticConstructObjectInternal,
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
            if let Ok(name) = Sig::from_str(&captures[1]) {
                let address = usize::from_str_radix(&captures[2], 16)?;
                if addresses.get(&name).map(|a| *a != address).unwrap_or(false) {
                    bail!("found multiple unique addresses for \"{}\"", name);
                }
                addresses.insert(name, address);
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
    #[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
    enum FNameToStringID {
        A,
        B,
    }
    impl FNameToStringID {
        fn resolve(&self, data: &[u8], base: usize, m: usize) -> usize {
            let n = (m - base).checked_add_signed(5).unwrap();
            base + n
                .checked_add_signed(i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize)
                .unwrap()
        }
    }
    #[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
    enum FNameFNameID {
        A,
        V5_1,
    }
    impl FNameFNameID {
        fn resolve(&self, data: &[u8], base: usize, m: usize) -> usize {
            match self {
                Self::A => {
                    let n = (m - base).checked_add_signed(0x18 + 5).unwrap();
                    base + n
                        .checked_add_signed(
                            i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                        )
                        .unwrap_or_default()
                }
                Self::V5_1 => {
                    let n = (m - base).checked_add_signed(0x1C + 5).unwrap();
                    base + n
                        .checked_add_signed(
                            i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                        )
                        .unwrap()
                }
            }
        }
    }
    #[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
    enum StaticConstructObjectInternalID {
        A,
        V4_12,
        V4_16_4_19,
        V5_0,
    }
    impl StaticConstructObjectInternalID {
        fn resolve(&self, data: &[u8], base: usize, m: usize) -> usize {
            match self {
                Self::A | Self::V4_12 => {
                    let n = m - base - 0x0e;
                    base + n
                        .checked_add_signed(
                            i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                        )
                        .unwrap()
                }
                Self::V4_16_4_19 | Self::V5_0 => {
                    let n = m - base + 5;
                    base + n
                        .checked_add_signed(
                            i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                        )
                        .unwrap_or_default()
                }
            }
        }
    }
    #[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
    enum GUObjectArrayID {
        A,
        V4_20,
    }
    impl GUObjectArrayID {
        fn resolve(&self, data: &[u8], base: usize, m: usize) -> usize {
            match self {
                Self::A => unimplemented!(),
                Self::V4_20 => {
                    let n = m - base + 3;
                    base + n
                        .checked_add_signed(
                            i32::from_le_bytes(data[n..n + 4].try_into().unwrap()) as isize
                        )
                        .unwrap()
                        - 0xc
                }
            }
        }
    }

    #[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
    enum PatternID {
        FNameToString(FNameToStringID),
        FNameFname(FNameFNameID),
        StaticConstructObjectInternal(StaticConstructObjectInternalID),
        GMalloc,
        GUObjectArray(GUObjectArrayID),
        GNatives,
    }
    impl PatternID {
        fn sig(&self) -> Sig {
            match self {
                Self::FNameToString(_) => Sig::FNameToString,
                Self::FNameFname(_) => Sig::FNameFName,
                Self::StaticConstructObjectInternal(_) => Sig::StaticConstructObjectInternal,
                Self::GMalloc => Sig::GMalloc,
                Self::GUObjectArray(_) => Sig::GUObjectArray,
                Self::GNatives => Sig::GNatives,
            }
        }
        fn resolve(&self, data: &[u8], base: usize, m: usize) -> usize {
            match self {
                Self::FNameToString(f) => f.resolve(data, base, m),
                Self::FNameFname(f) => f.resolve(data, base, m),
                Self::StaticConstructObjectInternal(f) => f.resolve(data, base, m),
                Self::GMalloc => m,
                Self::GUObjectArray(f) => f.resolve(data, base, m),
                Self::GNatives => {
                    for i in m - base..m - base + 400 {
                        if data[i] == 0x4c && data[i + 1] == 0x8d && data[i + 2] == 0x25 {
                            return (base + i + 7)
                                .checked_add_signed(i32::from_le_bytes(
                                    data[i + 3..i + 3 + 4].try_into().unwrap(),
                                ) as isize)
                                .unwrap_or_default();
                        }
                    }
                    0
                }
            }
        }
    }

    let patterns = [
        (
            PatternID::FNameToString(FNameToStringID::A),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9")?,
        ),
        (
            PatternID::FNameToString(FNameToStringID::B),
            Pattern::new("E8 ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E")?,
        ),

        (
            PatternID::FNameFname(FNameFNameID::A),
            Pattern::new("40 53 48 83 EC ?? 41 B8 01 00 00 00 48 8D 15 ?? ?? ?? ?? 48 8D 4C 24 ?? E8 ?? ?? ?? ?? B9")?
        ),
        (
            PatternID::FNameFname(FNameFNameID::V5_1),
            Pattern::new("57 48 83 EC 50 41 B8 01 00 00 00 0F 29 74 24 40 48 8D ?? ?? ?? ?? ?? 48 8D 4C 24 60 E8")?
        ),

        (
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::A),
            Pattern::new("C0 E9 02 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48")?,
        ),
        (
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V4_12),
            Pattern::new("89 8E C8 03 00 00 3B 8E CC 03 00 00 7E 0F 41 8B D6 48 8D 8E C0 03 00 00")?,
        ),
        (
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V4_16_4_19),
            Pattern::new("E8 ?? ?? ?? ?? 0F B6 8F ?? 01 00 00 48 89 87 ?? 01 00 00")?,
        ),
        (
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V5_0),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B D8 48 39 75 30 74 15")?,
        ) ,

        (
            PatternID::GMalloc,
            Pattern::new("48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? ?? ?? ?? ?? 48 85 C9")?,
        ),

        (
            PatternID::GUObjectArray(GUObjectArrayID::A),
            Pattern::new("48 03 ?? ?? ?? ?? ?? 48 8B 10 48 85 D2 74 07")?,
        ),
        (
            PatternID::GUObjectArray(GUObjectArrayID::V4_20),
            Pattern::new("48 8B ?? ?? ?? ?? ?? 48 8B 0C C8 ?? 8B 04 ?? 48 85 C0")?, // > 4.20
        ),

        (
            PatternID::GNatives,
            Pattern::new("cc 51 20 01")?,
        ),
    ];
    let pat: Vec<_> = patterns.iter().map(|(id, p)| (id, p)).collect();

    let games = fs::read_dir("games")?;

    'games: for entry in games {
        let entry = entry?;
        let dir_name = entry.file_name();
        let game = dir_name.to_string_lossy();
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
                continue 'games;
            }
        };

        let bin_data = fs::read(&exe_path)
            .with_context(|| format!("reading game exe {}", exe_path.display()))?;
        if let Some(log) = &log {
            if log.exe_size != bin_data.len() {
                println!("size mismatch: log indicates {} bytes but {} is {} bytes. is this the correct exe?", log.exe_size, exe_path.display(), bin_data.len());
                continue 'games;
            }
        }
        let obj_file = object::File::parse(&*bin_data)?;
        let exe_base = obj_file.relative_address_base() as usize;

        println!(
            "{} {} exe_base={:016x?}",
            game,
            exe_path.display(),
            exe_base,
        );

        struct Scan<'a> {
            base_address: usize,
            results: Vec<(&'a PatternID, usize)>,
        }

        let mut scans = vec![];

        for section in obj_file.sections() {
            if section.kind() != object::SectionKind::Text {
                continue;
            }

            let base_address = section.address() as usize;
            let data = section.data()?;
            scans.push(Scan {
                base_address,
                results: scan(pat.as_slice(), base_address, data)
                    .into_iter()
                    .map(|(id, m)| (id, id.resolve(data, base_address, m)))
                    .collect(),
            });
        }

        let folded_scans = scans
            .iter()
            .flat_map(|scan| scan.results.iter())
            .map(|(id, m)| (id.sig(), (id, *m)))
            .fold(HashMap::new(), |mut map, (k, v)| {
                map.entry(k).or_insert_with(Vec::new).push(v);
                map
            });

        use colored::Colorize;
        use itertools::join;
        use prettytable::{row, Table};

        let mut table = Table::new();
        table.set_titles(row!["sig", "log", "offline scan"]);

        for sig in Sig::iter() {
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
                            .fold(
                                HashMap::<&(&&PatternID, usize), usize>::new(),
                                |mut map, m| {
                                    *map.entry(m).or_default() += 1;
                                    map
                                }
                            )
                            .iter()
                            .map(|(m, count)| {
                                let count = if *count > 1 {
                                    format!(" (x{count})")
                                } else {
                                    "".to_string()
                                };
                                let s = format!("{:016x}{} {:?}", m.1, count, m.0);
                                if sig_log.is_none() {
                                    s.normal()
                                } else if m.1 == sig_log.unwrap() {
                                    s.green()
                                } else {
                                    s.red()
                                }
                            }),
                        "\n"
                    )
                    .normal())
                    .unwrap_or("not found".to_owned().red()),
            ]);
        }
        table.printstd();
        println!();
    }

    Ok(())
}
