use crate::symbols::dump_pdb_symbols;
use crate::{sift4_bin, MountedPE};

use anyhow::{Context, Result};
use byteorder::{ReadBytesExt, LE};
use itertools::Itertools;
use object::{Object, ObjectSection};

use std::collections::HashMap;
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

fn read_exe<'data>(
    data: &'data [u8],
    symbols: Option<&HashMap<u64, String>>,
) -> Result<Exe<'data>> {
    let object = object::File::parse(data)?;
    let memory = MountedPE::new(&object)?;
    let base_address = object.relative_address_base();
    let functions = read_exception_table(&object)?
        .into_iter()
        .map(|func| FunctionBody {
            body: memory.get_range(func.range.clone()),
            symbol: symbols.and_then(|s| s.get(&(func.range.start as u64 - base_address)).cloned()),
            func,
        })
        .collect::<Vec<_>>();

    Ok(Exe {
        object,
        memory,
        functions,
    })
}

fn read_exception_table(obj_file: &object::File) -> Result<Vec<RuntimeFunction>> {
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

struct Exe<'data> {
    object: object::File<'data>,
    memory: MountedPE<'data>,
    functions: Vec<FunctionBody<'data>>,
}

struct FunctionBody<'a> {
    func: RuntimeFunction,
    symbol: Option<String>,
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

fn bin_functions<'a>(fns: &'a Vec<FunctionBody<'a>>) -> HashMap<usize, Vec<&'a FunctionBody>> {
    let mut bins = HashMap::<usize, Vec<_>>::new();
    for f in fns {
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

pub fn functions(exe_path: std::path::PathBuf, other_exe_path: std::path::PathBuf) -> Result<()> {
    use rayon::prelude::*;

    let exe_data = std::fs::read(&exe_path)
        .with_context(|| format!("reading game exe {}", exe_path.display()))?;
    let exe = read_exe(&exe_data, None)?;

    let symbols = dump_pdb_symbols(other_exe_path.with_extension("pdb"))?;

    let other_exe_data = std::fs::read(&other_exe_path)
        .with_context(|| format!("reading game exe {}", other_exe_path.display()))?;
    let other_exe = read_exe(&other_exe_data, Some(&symbols))?;

    let bins = bin_functions(&other_exe.functions);

    use indicatif::ParallelProgressIterator;
    let records = exe
        .functions
        .par_iter()
        .progress_with_style(
            indicatif::ProgressStyle::with_template(
                "[{elapsed_precise}] [{wide_bar}] {pos}/{len} ({eta})",
            )
            .unwrap(),
        )
        //.take(100)
        .filter_map(|func| {
            //if of.1.len() < 1000 { return; }
            let m = bins
                .get(&bin(func.body))
                .map(|f| f.iter())
                .unwrap_or_default()
                .map(|other_func| {
                    let distance = sift4_bin::simple(func.body, other_func.body);
                    Diff {
                        a: func,
                        b: other_func,
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
                    symbol: m.b.symbol.clone(),
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
) -> Result<()> {
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
