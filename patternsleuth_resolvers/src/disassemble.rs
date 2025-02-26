use std::{collections::HashSet, ops::Range};

use iced_x86::{Decoder, DecoderOptions, FlowControl, Formatter, Instruction, NasmFormatter};

use crate::{Image, MemoryAccessError};

pub fn function_range(exe: &Image, address: usize) -> Result<Range<usize>, MemoryAccessError> {
    let min = address;
    let mut max = min;
    disassemble(exe, address, |inst| {
        let cur = inst.ip() as usize;
        if Some(address) != exe.get_root_function(cur)?.map(|f| f.range.start) {
            return Ok(Control::Break);
        }
        max = max.max(cur + inst.len());
        Ok(Control::Continue)
    })?;
    Ok(min..max)
}

pub fn disassemble_single(
    exe: &Image,
    address: usize,
) -> Result<Option<Instruction>, MemoryAccessError> {
    Ok(Decoder::with_ip(
        64,
        exe.memory.range_from(address..)?,
        address as u64,
        DecoderOptions::NONE,
    )
    .iter()
    .next())
}

pub enum Control {
    Continue,
    Break,
    Exit,
}

pub fn disassemble<F>(exe: &Image, address: usize, mut visitor: F) -> Result<(), MemoryAccessError>
where
    F: FnMut(&Instruction) -> Result<Control, MemoryAccessError>,
{
    struct Ctx<'data, 'img> {
        exe: &'img Image<'data>,
        queue: Vec<usize>,
        visited: HashSet<usize>,
        address: usize,
        block: &'img [u8], // these lifetimes are NOT 'data because of oddities with how Cow<'data> is handled in Image
        decoder: Decoder<'img>,
        instruction: Instruction,
    }

    let block = exe.memory.range_from(address..)?;
    let mut ctx = Ctx {
        exe,
        queue: Default::default(),
        visited: Default::default(),
        address,
        block,
        decoder: Decoder::with_ip(64, block, address as u64, DecoderOptions::NONE),
        instruction: Default::default(),
    };

    impl Ctx<'_, '_> {
        #[allow(unused)]
        fn print(&self) {
            let mut formatter = NasmFormatter::new();
            //formatter.options_mut().set_digit_separator("`");
            //formatter.options_mut().set_first_operand_char_index(10);

            let mut output = String::new();
            formatter.format(&self.instruction, &mut output);

            // Eg. "00007FFAC46ACDB2 488DAC2400FFFFFF     lea       rbp,[rsp-100h]"
            print!("{:016X} ", self.instruction.ip());
            let start_index = self.instruction.ip() as usize - self.address;
            let instr_bytes = &self.block[start_index..start_index + self.instruction.len()];
            for b in instr_bytes.iter() {
                print!("{:02X}", b);
            }
            if instr_bytes.len() < 0x10 {
                for _ in 0..0x10 - instr_bytes.len() {
                    print!("  ");
                }
            }
            println!(" {}", output);
        }
        fn start(&mut self, address: usize) -> Result<(), MemoryAccessError> {
            //println!("starting at {address:x}");
            self.address = address;
            self.block = self.exe.memory.range_from(self.address..)?;
            self.decoder =
                Decoder::with_ip(64, self.block, self.address as u64, DecoderOptions::NONE);
            Ok(())
        }
        /// Returns true if pop was successful
        fn pop(&mut self) -> Result<bool, MemoryAccessError> {
            Ok(if let Some(next) = self.queue.pop() {
                self.start(next)?;
                true
            } else {
                false
            })
        }
    }

    while ctx.decoder.can_decode() {
        ctx.decoder.decode_out(&mut ctx.instruction);

        let addr = ctx.instruction.ip() as usize;

        if ctx.visited.contains(&addr) {
            if ctx.pop()? {
                continue;
            } else {
                break;
            }
        } else {
            ctx.visited.insert(addr);
            match visitor(&ctx.instruction)? {
                Control::Continue => {}
                Control::Break => {
                    if ctx.pop()? {
                        continue;
                    } else {
                        break;
                    }
                }
                Control::Exit => {
                    break;
                }
            }
        }

        /*
        if !matches!(ctx.instruction.flow_control(), FlowControl::Next) {
            //println!();
        }
        ctx.print();
        if !matches!(ctx.instruction.flow_control(), FlowControl::Next) {
            //println!();
        }
        */

        match ctx.instruction.flow_control() {
            FlowControl::Next => {}
            FlowControl::UnconditionalBranch => {
                // TODO figure out how to handle tail calls
                ctx.start(ctx.instruction.near_branch_target() as usize)?;
            }
            //FlowControl::IndirectBranch => todo!(),
            FlowControl::ConditionalBranch => {
                ctx.queue
                    .push(ctx.instruction.near_branch_target() as usize);
            }
            FlowControl::Return => {
                if !ctx.pop()? {
                    break;
                }
            }
            //FlowControl::Call => todo!(),
            //FlowControl::IndirectCall => todo!(),
            //FlowControl::Interrupt => todo!(),
            //FlowControl::XbeginXabortXend => todo!(),
            //FlowControl::Exception => todo!(),
            _ => {}
        }
    }
    Ok(())
}
