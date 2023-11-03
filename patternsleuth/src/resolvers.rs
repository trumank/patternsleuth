use crate::{Memory, ResolutionAction, ResolutionType, ResolveContext, ResolveStages};

/// Simply return address of match
pub fn resolve_self(ctx: ResolveContext, _stages: &mut ResolveStages) -> ResolutionAction {
    ResolutionType::Address(ctx.match_address).into()
}

/// Return containing function via exception table lookup
pub fn resolve_function(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
    stages.0.push(ctx.match_address);
    ctx.exe
        .get_root_function(ctx.match_address)
        .map(|f| f.range.start)
        .into()
}

fn resolve_rip(
    memory: &Memory,
    match_address: usize,
    next_opcode_offset: usize,
    stages: &mut ResolveStages,
) -> ResolutionAction {
    stages.0.push(match_address);
    let rip_relative_value_address = match_address;
    // calculate the absolute address from the RIP relative value.
    let address = rip_relative_value_address
        .checked_add_signed(i32::from_le_bytes(
            memory[rip_relative_value_address..rip_relative_value_address + 4]
                .try_into()
                .unwrap(),
        ) as isize)
        .map(|a| a + next_opcode_offset);
    address.into()
}

/// Resolve RIP address at match, accounting for `N` bytes to the end of the instruction (usually 4)
pub fn resolve_rip_offset<const N: usize>(
    ctx: ResolveContext,
    stages: &mut ResolveStages,
) -> ResolutionAction {
    resolve_rip(ctx.memory, ctx.match_address, N, stages)
}
