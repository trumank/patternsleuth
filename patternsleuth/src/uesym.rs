use std::{collections::HashMap, path::Path};
use anyhow::{anyhow, Result};
use object::{from_bytes, slice_from_bytes, Pod};

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Header {
    record_count: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Record {
    address: u64,
    line_number: u32,
    file_relative_offset: u32,
    symbol_relative_offset: u32,
}

/// Safety: `Record` contains only a counter
/// and can be safely parsed from bytes.
unsafe impl Pod for Record {}

/// Safety: `Header` contains only a plain numbers
/// and offsets, thus it can be safely parsed from bytes.
unsafe impl Pod for Header {}

struct RawUESymbols<'data> {
    records: &'data [Record],
    address_map: HashMap<usize, usize>,
    data: &'data [u8],
}

struct WrapRecord<'a, 'data> {
    record: &'a Record,
    symbol: &'a RawUESymbols<'data>,
}

impl<'data> RawUESymbols<'data> {
    fn new(data: &'data [u8]) -> Result<RawUESymbols<'data>> {
        let (header, data) = from_bytes::<Header>(data).map_err(|_| anyhow!("Can't read haeder"))?;
        let (records, data) = 
            slice_from_bytes::<Record>(data, header.record_count as usize)
            .map_err(|_| anyhow!("Can't read Records"))?;
        let address_map: HashMap<usize, usize> = HashMap::from_iter(records.iter().enumerate().map(|(i, r)| (r.address as usize, i)));
        Ok(RawUESymbols {
            records,
            address_map,
            data,
        })
    }

    fn get_address(&self, address: usize) -> Option<WrapRecord<'_, 'data>> {
        let index = self.address_map.get(&address)?;
        Some(WrapRecord {
            record: &self.records[*index],
            symbol: self,
        })
    }

    fn iter(&self) -> impl Iterator<Item = WrapRecord<'_, 'data>> {
        self.records.iter().map(move |record| WrapRecord { record, symbol: self })
    }
}

impl WrapRecord<'_, '_> {
    fn read_str(&self, relative_offset: usize) -> &'_ str {
        let start = relative_offset;
        let end = self.symbol.data[start..].iter().position(|&b| b == 0 || b == '\n' as _).unwrap();
        std::str::from_utf8(&self.symbol.data[start..start + end]).unwrap()
    }

    fn symbol(&self) -> &'_ str {
        self.read_str(self.record.symbol_relative_offset as usize)
    }

    fn filename(&self) -> &'_ str {
        self.read_str(self.record.file_relative_offset as usize)
    }
}

pub fn dump_ue_symbols<P: AsRef<Path>>(
    filename: P,
    base_address: usize,
) -> Result<HashMap<usize, String>> {
    let data = std::fs::read(filename)?;
    let symbols = RawUESymbols::new(data.as_slice())?;
    Ok(HashMap::from_iter(symbols.iter().map(|rec| {
        (rec.record.address as usize, rec.symbol().to_string())
    })))
}
