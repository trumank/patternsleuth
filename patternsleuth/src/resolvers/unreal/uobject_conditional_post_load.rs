use crate::MemoryTrait;
use crate::resolvers::{ensure_one, impl_resolver_singleton, unreal::util};
use futures::future::join_all;
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use itertools::Itertools;
use patternsleuth_scanner::Pattern;

fn is_scratch_register(register: Register) -> bool {
    register == Register::RAX
        && register == Register::RCX
        && register == Register::RDX
        && register == Register::R8
        && register == Register::R9
}

fn match_postloading_assignments_tail(instructions: &Vec<Instruction>, local_index: &mut usize, currently_post_loaded_object_by_alt: &Option<(Register, u32)>, lea_storage_register: Register, object_source_register: Register) -> bool {
    // Optional: mov qword [rdi+0x90], $zeroed_register
    if *local_index + 1 < instructions.len() && instructions[*local_index].mnemonic() == Mnemonic::Mov && instructions[*local_index].op0_kind() == OpKind::Memory && instructions[*local_index].op1_kind() == OpKind::Register && let Some((base_register, displacement)) = currently_post_loaded_object_by_alt && instructions[*local_index].memory_base() == *base_register && instructions[*local_index].memory_displacement32() == *displacement {
        *local_index += 1;
    }

    // mov qword [rdi+0x90], $object_source_register
    if *local_index + 1 < instructions.len() && instructions[*local_index].mnemonic() == Mnemonic::Mov && instructions[*local_index].op0_kind() == OpKind::Memory && instructions[*local_index].op1_kind() == OpKind::Register && instructions[*local_index].op1_register() == object_source_register {
        let async_package_register = instructions[*local_index].memory_base();
        *local_index += 1;

        // mov qword [rdi+0x98], $lea_storage_register
        if !is_scratch_register(async_package_register) && instructions[*local_index].mnemonic() == Mnemonic::Mov && instructions[*local_index].op0_kind() == OpKind::Memory && instructions[*local_index].op1_kind() == OpKind::Register && instructions[*local_index].op1_register() == lea_storage_register && instructions[*local_index].memory_base() == async_package_register {
            return true;
        }
    }
    false
}

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UObjectConditionalPostLoad(pub u64);
impl_resolver_singleton!(all, UObjectConditionalPostLoad, |ctx| async {

    let strings = join_all([
        ctx.scan(util::utf16_pattern("postloading\0")), // 4.0-4.7
        ctx.scan(util::utf16_pattern("postloading_async\0")) // 4.8+
    ]).await.into_iter().flatten().collect_vec();

    #[derive(PartialEq, Eq)] enum V { Indirect, Direct }
    let patterns = strings.into_iter().flat_map(|str_addr| [
        (V::Indirect, format!("48 8d ?? X0x{str_addr:08x}")),
        (V::Indirect, format!("4c 8d ?? X0x{str_addr:08x}")),
        (V::Direct, format!("48 8b cf 49 89 7e ?? e8 | ?? ?? ?? ?? 4d 89 6e ?? 48 8d 05 X0x{str_addr:08x}")),
    ].into_iter()).collect_vec();
    let match_result = join_all(patterns.iter().map(|(v, p)| ctx.scan_tagged(v, Pattern::new(p).unwrap()))).await;

    let mut conditional_post_load_addresses = Vec::new();

    for (v, _, addresses) in match_result {
        for a in addresses {
            if *v == V::Indirect {
                let function_disassembly_length = 0x260; // 600 bytes should be enough for that function even when AtomicSetFlags is present
                let bytes = ctx.image().memory.range(a..a + function_disassembly_length)?;
                let instructions = Decoder::with_ip(64, bytes, a, DecoderOptions::NONE).iter().collect_vec();
                let mut candidates: Vec<(Register, u64, Option<(Register, u32)>)> = Vec::new();

                let mut index = 0;
                let lea_instruction = instructions[index];
                index += 1;
                assert_eq!(lea_instruction.mnemonic(), Mnemonic::Lea);
                let lea_storage_register = lea_instruction.op0_register();

                // Lea must not relate to a temporary register for the indirect pattern to make sense
                if !is_scratch_register(lea_storage_register) {
                    while index < instructions.len() {
                        let mut local_index = index;

                        // We are looking for something similar to this here:
                        // mov rcx, rbx
                        if instructions[local_index].mnemonic() == Mnemonic::Mov {
                            if local_index + 1 < instructions.len() && instructions[local_index].op0_kind() == OpKind::Register &&
                            instructions[local_index].op0_register() == Register::RCX && instructions[local_index].op1_kind() == OpKind::Register {
                                let object_source_register = instructions[local_index].op1_register();
                                local_index += 1;

                                // Some UE versions (most of them actually) have CurrentlyPostLoadedObjectByALT being set before and after the call
                                // mov qword [mem], $object_source_register
                                let mut currently_post_loaded_object_by_alt: Option<(Register, u32)> = None;
                                if local_index + 1 < instructions.len() && instructions[local_index].mnemonic() == Mnemonic::Mov && instructions[local_index].op0_kind() == OpKind::Memory &&
                                instructions[local_index].op1_kind() == OpKind::Register &&
                                    instructions[local_index].op1_register() == object_source_register {
                                    currently_post_loaded_object_by_alt = Some((instructions[local_index].memory_base(), instructions[local_index].memory_displacement32()));
                                    local_index += 1;
                                }

                                // call UObject::ConditionalPostLoad
                                if local_index + 1 < instructions.len() && instructions[local_index].mnemonic() == Mnemonic::Call &&
                                instructions[local_index].op0_kind() == OpKind::NearBranch64 && !is_scratch_register(object_source_register) {
                                    let call_target_address = instructions[local_index].near_branch64();
                                    local_index += 1;

                                    // Try to match the tail now
                                    if match_postloading_assignments_tail(&instructions, &mut local_index, &currently_post_loaded_object_by_alt, lea_storage_register, object_source_register) {
                                        // We matched the tail, this is UObject::ConditionalPostLoad
                                        conditional_post_load_addresses.push(call_target_address);
                                        break;
                                    } else {
                                        // In 4.24-4.25, we will encounter AtomicallyClearInternalFlags(EInternalObjectFlags::AsyncLoading) call here before our expected assignments
                                        // It can be easily distinguished by a 4-byte read from the object pointer to eax
                                        // mov eax, dword [rsi+InternalIndex]
                                        if instructions[local_index].mnemonic() == Mnemonic::Mov && instructions[local_index].op0_kind() == OpKind::Register &&
                                        instructions[local_index].op0_register().size() == 4 && instructions[local_index].op1_kind() == OpKind::Memory && instructions[local_index].memory_base() == object_source_register {
                                            // Write down information about a possible later tail and keep processing instructions
                                            candidates.push((object_source_register, call_target_address, currently_post_loaded_object_by_alt));
                                            index = local_index;
                                            continue;
                                        }
                                    }
                                }
                            } else if instructions[local_index].op0_kind() == OpKind::Memory {
                                // Try to match this as a tail against all possible candidates
                                let mut should_break_from_outer_loop = false;
                                for candidate in &candidates {
                                    if match_postloading_assignments_tail(&instructions, &mut local_index, &candidate.2, lea_storage_register, candidate.0) {
                                        // We matched the tail for one of the candidates
                                        conditional_post_load_addresses.push(candidate.1);
                                        should_break_from_outer_loop = true;
                                        break;
                                    }
                                }
                                if should_break_from_outer_loop {
                                    break;
                                }
                            }
                        }

                        // Advance the instruction pointer
                        index += 1;
                    }
                }
            } else {
                conditional_post_load_addresses.push(ctx.image().memory.rip4(a)?);
            }
        }
    }

    Ok(Self(ensure_one(conditional_post_load_addresses)?))
});
