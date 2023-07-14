use std::{collections::HashMap, path::Path};

use pdb::{FallibleIterator, PdbInternalSectionOffset};

fn print_row(
    symbols: &mut HashMap<u64, String>,
    address_map: &pdb::AddressMap<'_>,
    base_address: u64,
    offset: PdbInternalSectionOffset,
    name: pdb::RawString<'_>,
) {
    let name = name.to_string().to_string();
    if let Some(rva) = offset.to_rva(address_map) {
        symbols.insert(rva.0 as u64 + base_address, name);
    } else {
        println!("failed to calc RVA for {}", name);
    }
}

fn print_symbol(
    symbols: &mut HashMap<u64, String>,
    address_map: &pdb::AddressMap<'_>,
    base_address: u64,
    symbol: &pdb::Symbol<'_>,
) -> pdb::Result<()> {
    match symbol.parse()? {
        pdb::SymbolData::Public(data) => {
            print_row(symbols, address_map, base_address, data.offset, data.name);
        }
        pdb::SymbolData::Procedure(data) => {
            print_row(symbols, address_map, base_address, data.offset, data.name);
        }
        _ => {}
    }

    Ok(())
}

fn walk_symbols(
    symbols_map: &mut HashMap<u64, String>,
    address_map: &pdb::AddressMap<'_>,
    base_address: u64,
    mut symbols: pdb::SymbolIter<'_>,
) -> pdb::Result<()> {
    println!("segment\toffset\tkind\tname");

    while let Some(symbol) = symbols.next()? {
        match print_symbol(symbols_map, address_map, base_address, &symbol) {
            Ok(_) => (),
            Err(_e) => {
                //eprintln!("error printing symbol {:?}: {}", symbol, e);
            }
        }
    }

    Ok(())
}

pub fn dump_pdb_symbols<P: AsRef<Path>>(filename: P, base_address: u64) -> pdb::Result<HashMap<u64, String>> {
    let mut symbols = HashMap::new();

    let file = std::fs::File::open(filename)?;
    let mut pdb = pdb::PDB::open(file)?;
    let symbol_table = pdb.global_symbols()?;
    let address_map = pdb.address_map()?;
    println!("Global symbols:");
    walk_symbols(
        &mut symbols,
        &address_map,
        base_address,
        symbol_table.iter(),
    )?;

    println!("Module private symbols:");
    let dbi = pdb.debug_information()?;
    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        println!("Module: {}", module.object_file_name());
        let info = match pdb.module_info(&module)? {
            Some(info) => info,
            None => {
                println!("  no module info");
                continue;
            }
        };

        walk_symbols(&mut symbols, &address_map, base_address, info.symbols()?)?;
    }
    Ok(symbols)
}
