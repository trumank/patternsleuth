use std::ops::Range;

use colored::{ColoredString, Colorize};
use iced_x86::{
    Decoder, DecoderOptions, Formatter, FormatterOutput, FormatterTextKind, IntelFormatter, OpKind,
};
use patternsleuth::{image::Image, scanner::Pattern, MemoryTrait};

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

pub(crate) fn disassemble(exe: &Image, address: usize, pattern: Option<&Pattern>) -> String {
    let context = 20; // number of instructions before and after
    let max_inst = 16; // max size of x86 instruction in bytes

    let mut output = Output::default();

    if let Ok(section) = exe.memory.get_section_containing(address) {
        output.buffer.push_str(&format!(
            "{:016x}\n{:016x} - {:016x} = {}\n",
            address,
            section.address(),
            section.address() + section.data().len(),
            section.name(),
        ));

        let (is_fn, data, start_address) = if let Ok(Some(f)) = exe.get_root_function(address) {
            let fns = exe.get_child_functions(f.range.start).unwrap();
            let min = fns.iter().map(|f| f.range.start).min().unwrap();
            let max = fns.iter().map(|f| f.range.end).max().unwrap();
            let range = min..max;

            output.buffer.push_str(&format!(
                "{:016x} - {:016x} = function\n",
                range.start, range.end
            ));
            if let Some(symbols) = &exe.symbols {
                if let Some(symbol) = symbols.get(&range.start) {
                    #[allow(clippy::unnecessary_to_owned)]
                    output
                        .buffer
                        .push_str(&symbol.name.bright_yellow().to_string());
                    output.buffer.push_str(&"".normal().to_string());
                    output.buffer.push('\n');
                }
            }
            let start_address = range.start as u64;
            let data = section.range(range).unwrap();
            (true, data, start_address)
        } else {
            output.buffer.push_str("no function");

            let data = &section.data()[(address - context * max_inst)
                .saturating_sub(section.address())
                ..(address + context * max_inst).saturating_sub(section.address())];
            let start_address = (address - context * max_inst) as u64;
            (false, data, start_address)
        };

        output.buffer.push('\n');

        let mut decoder = Decoder::with_ip(64, data, start_address, DecoderOptions::NONE);

        let instructions = decoder.iter().collect::<Vec<_>>();
        let instructions = if let Some((middle, _)) = (!is_fn)
            .then(|| {
                instructions
                    .iter()
                    .enumerate()
                    .find(|(_, inst)| inst.ip() >= address as u64)
            })
            .flatten()
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
                        let offset = (instruction.ip() as usize) - address + i + p.custom_offset;
                        Some(*p.simple.mask.get(offset)? != 0)
                    })
                    .unwrap_or_default();
                let s = format!("{b:02x}");
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
            .push_str(&format!("{address:016x}\nno section"));
    }
    output.buffer
}

pub(crate) fn disassemble_range(exe: &Image, range: Range<usize>) -> String {
    let address = range.start;
    let mut output = Output::default();

    if let Ok(section) = exe.memory.get_section_containing(address) {
        let data = &section.range(range).unwrap();

        output.buffer.push_str(&format!(
            "{:016x}\n{:016x} - {:016x} = {}\n",
            address,
            section.address(),
            section.address() + section.data().len(),
            section.name(),
        ));

        if let Ok(Some(f)) = exe.get_root_function(address) {
            output.buffer.push_str(&format!(
                "{:016x} - {:016x} = function\n",
                f.range.start, f.range.end
            ));
            if let Some(symbols) = &exe.symbols {
                if let Some(symbol) = symbols.get(&f.range.start) {
                    #[allow(clippy::unnecessary_to_owned)]
                    output
                        .buffer
                        .push_str(&format!("{}\n", symbol.name).bright_yellow().to_string());
                }
            }
        } else {
            output.buffer.push_str("no function");
        }

        output.buffer.push('\n');

        let mut decoder = Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE);

        let instructions = decoder.iter().collect::<Vec<_>>();

        let mut formatter = IntelFormatter::new();
        formatter.options_mut().set_first_operand_char_index(8);
        for instruction in instructions {
            let ip = format!("{:016x}", instruction.ip());
            output.buffer.push_str(&ip);
            output.buffer.push_str(":  ");

            let index = instruction.ip() as usize - address;
            for b in data[index..index + instruction.len()].iter() {
                let s = format!("{b:02x}");
                #[allow(clippy::unnecessary_to_owned)]
                output.buffer.push_str(&s.bright_white().to_string());
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
            .push_str(&format!("{address:016x}\nno section"));
    }
    output.buffer
}

pub(crate) fn disassemble_bytes_with_symbols<F>(
    address: usize,
    data: &[u8],
    pattern: Option<&Pattern>,
    symbols: F,
) -> String
where
    F: Fn(usize) -> Option<String>,
{
    let mut output = Output::default();

    output.buffer.push_str(&format!(
        "{:016x} - {:016x}\n",
        address,
        address + data.len()
    ));

    if let Some(symbol) = symbols(address) {
        #[allow(clippy::unnecessary_to_owned)]
        output
            .buffer
            .push_str(&format!("{}", symbol.bright_yellow().to_owned()));
    }

    output.buffer.push_str("\n\n");

    let mut formatter = IntelFormatter::new();
    formatter.options_mut().set_first_operand_char_index(8);
    for instruction in Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE) {
        let ip = format!("{:016x}", instruction.ip());
        output.buffer.push_str(&ip);
        output.buffer.push_str(":  ");

        let index = instruction.ip() as usize - address;
        for (i, b) in data[index..index + instruction.len()].iter().enumerate() {
            let highlight = pattern
                .and_then(|p| -> Option<bool> {
                    let offset = (instruction.ip() as usize) - address + i + p.custom_offset;
                    Some(*p.simple.mask.get(offset)? != 0)
                })
                .unwrap_or_default();

            let s = format!("{b:02x}");
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

        if instruction.op_kinds().any(|op| op == OpKind::NearBranch64) {
            if let Some(symbol) = symbols(instruction.near_branch64() as usize) {
                #[allow(clippy::unnecessary_to_owned)]
                output
                    .buffer
                    .push_str(&format!(" {}", symbol.bright_yellow().to_owned()));
            }
        }
        output.buffer.push('\n');
    }
    output.buffer
}

pub(crate) fn get_xrefs(address: usize, data: &[u8]) -> Vec<(usize, usize)> {
    let mut xrefs = vec![];
    for instruction in Decoder::with_ip(64, data, address as u64, DecoderOptions::NONE) {
        if instruction.op_kinds().any(|op| op == OpKind::NearBranch64) {
            xrefs.push((
                instruction.ip() as usize,
                instruction.near_branch64() as usize,
            ));
        }
    }
    xrefs
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
