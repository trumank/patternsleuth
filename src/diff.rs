use crate::symbols::dump_pdb_symbols;
use crate::{sift4_bin, MountedPE};

use anyhow::{Context, Result};
use byteorder::{ReadBytesExt, LE};
use itertools::Itertools;
use object::{Object, ObjectSection};

use std::collections::HashMap;
use std::error::Error;
use std::ops::Range;
use std::{io::BufRead, io::Read};

#[derive(Debug, Clone)]
struct RuntimeFunction {
    range: Range<usize>,
    _unwind: usize,
}
impl RuntimeFunction {
    fn read(base_address: usize, data: &mut impl Read) -> Result<Self> {
        Ok(Self {
            range: base_address + data.read_u32::<LE>()? as usize
                ..base_address + data.read_u32::<LE>()? as usize,
            _unwind: base_address + data.read_u32::<LE>()? as usize,
        })
    }
}

fn serialize_hex<S>(v: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format!("{:x}", v))
}
fn deserialize_hex<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let s: &str = serde::Deserialize::deserialize(deserializer)?;
    u64::from_str_radix(s, 16).map_err(D::Error::custom)
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DiffRecord {
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    a_start: u64,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    a_end: u64,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    b_start: u64,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    b_end: u64,
    symbol: Option<String>,
    distance: i32,
}

fn read_exe(obj_file: &object::File) -> Result<Vec<RuntimeFunction>, Box<dyn Error>> {
    use std::io::Cursor;

    let exe_base = obj_file.relative_address_base() as usize;

    let mut pdata = Cursor::new(
        obj_file
            .section_by_name(".pdata")
            .context(".pdata section does not exist")?
            .data()?,
    );

    Ok(std::iter::from_fn(|| RuntimeFunction::read(exe_base, &mut pdata).ok()).collect())
}

struct FunctionBody<'a> {
    func: &'a RuntimeFunction,
    body: &'a [u8],
}
struct Diff<'a> {
    a: &'a FunctionBody<'a>,
    b: &'a FunctionBody<'a>,
    distance: i32,
}

const S: f32 = 30.0;
fn bin(s: &[u8]) -> usize {
    ((s.len() as f32).log10() * S) as usize
}
fn inv_bin(b: usize) -> usize {
    10f32.powf(b as f32 / S) as usize
}

fn bin_functions<'a>(fn_dis: &'a Vec<FunctionBody<'a>>) -> HashMap<usize, Vec<&'a FunctionBody>> {
    let mut bins = HashMap::<usize, Vec<_>>::new();
    for f in fn_dis {
        let i = bin(f.body);
        bins.entry(i - 1).or_default().push(f);
        bins.entry(i).or_default().push(f);
        bins.entry(i + 1).or_default().push(f);
    }
    for (k, v) in bins.iter().sorted_by_key(|e| e.0) {
        println!("{} ({}): {}", k, inv_bin(*k), v.len());
    }
    bins
}

pub fn functions(
    exe: std::path::PathBuf,
    other_exe: std::path::PathBuf,
) -> Result<(), Box<dyn Error>> {
    use rayon::prelude::*;

    let exe_data =
        std::fs::read(&exe).with_context(|| format!("reading game exe {}", exe.display()))?;
    let exe_obj = object::File::parse(&*exe_data)?;
    let memory = MountedPE::new(&exe_obj)?;

    let other_exe_data = std::fs::read(&other_exe)
        .with_context(|| format!("reading game exe {}", other_exe.display()))?;
    let other_exe_obj = object::File::parse(&*other_exe_data)?;
    let other_memory = MountedPE::new(&other_exe_obj)?;

    let functions = read_exe(&exe_obj)?;
    let other_functions = read_exe(&other_exe_obj)?;

    let symbols = dump_pdb_symbols(
        other_exe.with_extension("pdb"),
        other_exe_obj.relative_address_base(),
    )?;

    println!("disassembling {}", exe.display());
    let fn_dis = functions
        .par_iter()
        .map(|func| FunctionBody {
            func,
            body: &memory[func.range.clone()],
        })
        .collect::<Vec<_>>();
    println!("disassembling {}", other_exe.display());
    let other_fn_dis = other_functions
        .par_iter()
        .map(|func| FunctionBody {
            func,
            body: &other_memory[func.range.clone()],
        })
        .collect::<Vec<_>>();

    let bins = bin_functions(&fn_dis);

    use indicatif::ParallelProgressIterator;
    let records = other_fn_dis
        .par_iter()
        .progress_with_style(
            indicatif::ProgressStyle::with_template(
                "[{elapsed_precise}] [{wide_bar}] {pos}/{len} ({eta})",
            )
            .unwrap(),
        ) //.take(10)
        .filter_map(|of| {
            //if of.1.len() < 1000 { return; }
            let m = bins
                .get(&bin(of.body))
                .map(|f| f.iter())
                .unwrap_or_default()
                .map(|f| {
                    let distance = sift4_bin::simple(of.body, f.body);
                    Diff {
                        a: f,
                        b: of,
                        distance,
                    }
                })
                .min_by_key(|f| f.distance);

            if let Some(m) = m {
                return Some(DiffRecord {
                    a_start: m.a.func.range.start as u64,
                    a_end: m.a.func.range.end as u64,
                    b_start: m.b.func.range.start as u64,
                    b_end: m.b.func.range.end as u64,
                    symbol: symbols.get(&(of.func.range.start as u64)).cloned(),
                    distance: m.distance,
                });
            }
            None
        })
        .collect::<Vec<_>>();

    let mut wtr = csv::Writer::from_path("diff.csv")?;
    for record in records {
        wtr.serialize(record)?;
    }
    wtr.flush()?;

    Ok(())
}

pub fn sym(
    exe: std::path::PathBuf,
    other_exe: std::path::PathBuf,
    address: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let mut rdr = csv::Reader::from_path("diff.csv")?;
    let records = rdr
        .deserialize()
        .map(|r| Ok(r?))
        .collect::<Result<Vec<DiffRecord>>>()?;

    fn find_record_containing(records: &[DiffRecord], address: u64) -> Option<&DiffRecord> {
        records
            .iter()
            .find(|r| r.a_start <= address && r.a_end > address)
    }

    if let Some(address) = address {
        if let Ok(address) = u64::from_str_radix(&address, 16) {
            for record in records {
                if record.a_start <= address && record.a_end > address {
                    println!("{:#?}", record);
                }
            }
        } else {
            let file = std::fs::File::open(address)?;
            let lines = std::io::BufReader::new(file).lines();

            use regex::Regex;
            let re = Regex::new(r#"(?<start>LogWindows: Error: \[Callstack\] 0x(?<address>[0-9a-fA-F]+) FSD-Win64-Shipping.exe!)(?<symbol>UnknownFunction)(?<end> \[\])"#).unwrap();
            //let result = re.replace_all("Hello World!", "x");
            //println!("{}", result); // => "xxxxx xxxxx!"

            use regex::{Captures, Replacer};

            struct SymbolResolver<'re>(&'re [DiffRecord]);

            impl<'re> Replacer for SymbolResolver<'re> {
                fn replace_append(&mut self, caps: &Captures<'_>, dst: &mut String) {
                    dst.push_str(&caps["start"]);
                    if let Some(DiffRecord {
                        a_start,
                        a_end,
                        distance: score,
                        symbol: Some(symbol),
                        ..
                    }) = find_record_containing(
                        self.0,
                        u64::from_str_radix(&caps["address"], 16).unwrap(),
                    ) {
                        dst.push_str(&format!(
                            "{:x} @ {:_>5}/{:_>5} {}",
                            a_start,
                            score,
                            a_end - a_start,
                            symbol
                        ));
                    } else {
                        dst.push_str(&caps["symbol"]);
                    }
                    dst.push_str(&caps["end"]);
                }
            }

            for line in lines {
                let line = line?;
                println!("{}", re.replace(&line, SymbolResolver(&records)));
            }
        }
    }

    Ok(())
}
