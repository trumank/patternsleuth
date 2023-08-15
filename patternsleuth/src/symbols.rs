use anyhow::Result;

use std::{collections::HashMap, path::Path};

use pdb::FallibleIterator;

fn demangle(name: pdb::RawString<'_>) -> String {
    let name = name.to_string().to_string();
    msvc_demangler::demangle(&name, msvc_demangler::DemangleFlags::llvm())
        .unwrap_or_else(|_| name.to_string())
}

fn print_symbol(
    symbols: &mut HashMap<usize, String>,
    address_map: &pdb::AddressMap<'_>,
    base_address: usize,
    symbol: &pdb::Symbol<'_>,
) -> pdb::Result<()> {
    #[allow(clippy::single_match)]
    match symbol.parse()? {
        pdb::SymbolData::Public(data) => {
            let name_demangled = demangle(data.name);
            if let Some(rva) = data.offset.to_rva(address_map) {
                let address = base_address + rva.0 as usize;
                symbols.insert(address, name_demangled);
            }
        }
        // procedure symbols don't seem to always be availble so instead we use the exception table to get the function bounds
        /*
        pdb::SymbolData::Procedure(data) => {
            let name_demangled = demangle(data.name);
            if filter(&name_demangled) {
                if let Some(rva) = data.offset.to_rva(address_map) {
                    println!(
                        "{:016x} proc len={:08x} {}",
                        rva.0, data.len, name_demangled
                    );
                }
            }
        }
        */
        _ => {}
    }

    Ok(())
}

fn walk_symbols(
    symbols_map: &mut HashMap<usize, String>,
    address_map: &pdb::AddressMap<'_>,
    base_address: usize,
    mut symbols: pdb::SymbolIter<'_>,
) -> pdb::Result<()> {
    while let Some(symbol) = symbols.next()? {
        print_symbol(symbols_map, address_map, base_address, &symbol).ok();
    }
    Ok(())
}

pub fn dump_pdb_symbols<P: AsRef<Path>>(
    filename: P,
    base_address: usize,
) -> Result<HashMap<usize, String>> {
    let mut symbols = HashMap::new();

    let file = std::fs::File::open(filename)?;
    let mut pdb = pdb::PDB::open(file)?;
    let symbol_table = pdb.global_symbols()?;
    let address_map = pdb.address_map()?;
    walk_symbols(
        &mut symbols,
        &address_map,
        base_address,
        symbol_table.iter(),
    )?;

    let dbi = pdb.debug_information()?;
    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        let Some(info) = pdb.module_info(&module)? else {
            continue;
        };
        walk_symbols(&mut symbols, &address_map, base_address, info.symbols()?)?;
    }
    Ok(symbols)
}
