use crate::{LinearOp as Op, ScalarProgramBlock, SolvePureCallTable, TensorInputKind};

pub(super) fn outputs(
    table: &SolvePureCallTable,
    block: &ScalarProgramBlock,
    y: &[bool],
    p: &[bool],
) -> Option<Vec<bool>> {
    let mut outputs = Vec::new();
    for program in block.programs() {
        outputs.extend(program_outputs(table, program, y, p)?);
    }
    Some(outputs)
}

pub(in crate::fmi) fn program_outputs(
    table: &SolvePureCallTable,
    program: &[Op],
    y: &[bool],
    p: &[bool],
) -> Option<Vec<bool>> {
    region_outputs(table, program, y, p, Frame::default())
}

#[derive(Clone, Copy, Default)]
struct Frame<'a> {
    conditional: &'a [bool],
    carried: &'a [bool],
    captures: &'a [bool],
}

/// [`program_outputs`] of a program whose function-conditional capture loads
/// read `captures`.
fn region_outputs(
    table: &SolvePureCallTable,
    program: &[Op],
    y: &[bool],
    p: &[bool],
    frame: Frame<'_>,
) -> Option<Vec<bool>> {
    let mut outputs = Vec::new();
    let mut registers = Vec::new();
    for op in program {
        let end = op
            .dst_register()
            .map_or(0, |dst| dst as usize + op.dst_register_count());
        registers.resize(registers.len().max(end), false);
        frame_transfer(table, op, &mut registers, y, p, frame, &mut outputs)?;
    }
    Some(outputs)
}

// SPEC_0021: Exception - dispatch over scalar operations with nested region frames.
#[allow(clippy::too_many_lines)]
fn frame_transfer(
    table: &SolvePureCallTable,
    op: &Op,
    registers: &mut [bool],
    y: &[bool],
    p: &[bool],
    frame: Frame<'_>,
    outputs: &mut Vec<bool>,
) -> Option<()> {
    match op {
        Op::LoadFunctionConditionalCapture { dst, index } => {
            registers[*dst as usize] = *frame.conditional.get(*index)?;
        }
        Op::LoadFunctionConditionalCaptureRange {
            dst_start,
            index_start,
            count,
        } => registers
            .get_mut(*dst_start as usize..*dst_start as usize + count)?
            .copy_from_slice(frame.conditional.get(*index_start..*index_start + count)?),
        Op::FunctionConditional {
            dst_start,
            capture_start,
            program,
        } => {
            // Settled when every region settles over the settled captures.
            let inner = registers
                .get(*capture_start as usize..*capture_start as usize + program.capture_count)?
                .to_vec();
            let mut settled = true;
            for region in program
                .arms
                .iter()
                .flat_map(|arm| [&arm.condition, &arm.result])
                .chain(std::iter::once(&program.fallback))
            {
                settled &= region_outputs(
                    table,
                    region,
                    y,
                    p,
                    Frame {
                        conditional: &inner,
                        ..frame
                    },
                )?
                .iter()
                .all(|value| *value);
            }
            registers
                .get_mut(*dst_start as usize..*dst_start as usize + program.result_count)?
                .fill(settled);
        }
        Op::LoadFoldCarried { dst, index } => {
            registers[*dst as usize] = *frame.carried.get(*index)?
        }
        Op::LoadFoldCapture { dst, index } => {
            registers[*dst as usize] = *frame.captures.get(*index)?
        }
        Op::LoadFoldIndex { dst, .. } => registers[*dst as usize] = true,
        Op::LoadIndexedFoldCarried {
            dst,
            base,
            stride,
            dimensions,
            indices,
        } => {
            registers[*dst as usize] = indexed(
                frame.carried,
                *base,
                *stride,
                dimensions,
                indices,
                registers,
            )?;
        }
        Op::LoadIndexedFoldCapture {
            dst,
            base,
            stride,
            dimensions,
            indices,
        } => {
            registers[*dst as usize] = indexed(
                frame.captures,
                *base,
                *stride,
                dimensions,
                indices,
                registers,
            )?;
        }
        Op::FunctionFold {
            dst_start,
            initial_start,
            capture_start,
            program,
        }
        | Op::GuardedFunctionFold {
            dst_start,
            initial_start,
            capture_start,
            program,
            ..
        } => {
            let mut values = fold_outputs(
                table,
                program,
                registers,
                *initial_start,
                *capture_start,
                y,
                p,
            )?;
            if let Op::GuardedFunctionFold { activation, .. } = op {
                values
                    .iter_mut()
                    .for_each(|value| *value &= registers[*activation as usize]);
            }
            registers
                .get_mut(*dst_start as usize..*dst_start as usize + values.len())?
                .copy_from_slice(&values);
        }
        Op::StoreOutputFoldTensorUpdate { .. } => {
            outputs.extend(tensor_update_outputs(op, registers, frame)?);
        }
        Op::StoreOutputFunctionFold {
            initial,
            capture_start,
            program,
            result_base,
            count,
            condition,
            ..
        } => {
            let mut entry = Vec::new();
            for source in initial {
                match source {
                    crate::FoldInitialSource::Registers { start, count } => entry
                        .extend_from_slice(
                            registers.get(*start as usize..*start as usize + count)?,
                        ),
                    crate::FoldInitialSource::ParentCarried { base, count } => {
                        entry.extend_from_slice(frame.carried.get(*base..*base + count)?)
                    }
                }
            }
            let captures = registers
                .get(*capture_start as usize..*capture_start as usize + program.capture_count)?;
            let values = fold_frame_outputs(table, program, entry.clone(), captures, y, p)?;
            let stable_guard = condition.is_none_or(|id| registers[id as usize]);
            outputs.extend(
                values
                    .get(*result_base..*result_base + count)?
                    .iter()
                    .zip(entry.get(*result_base..*result_base + count)?)
                    .map(|(value, initial)| stable_guard && *value && *initial),
            );
        }
        _ => transfer(table, op, registers, y, p, outputs)?,
    }
    Some(())
}

fn tensor_update_outputs(op: &Op, registers: &[bool], frame: Frame<'_>) -> Option<Vec<bool>> {
    let Op::StoreOutputFoldTensorUpdate {
        source_base,
        source_stride,
        dimensions,
        updates,
        nodes,
        lanes,
        ..
    } = op
    else {
        return None;
    };

    let count = dimensions.iter().map(|n| *n as usize).product::<usize>();
    let mut stable = (0..count).all(|i| {
        frame
            .carried
            .get(source_base + i * source_stride..source_base + i * source_stride + lanes)
            .is_some_and(|values| values.iter().all(|v| *v))
    });
    for update in updates {
        let mut elements = 1;
        for (subscript, extent) in update.subscripts.iter().zip(dimensions) {
            match subscript {
                crate::TensorSubscript::Whole => elements *= *extent as usize,
                crate::TensorSubscript::Index(crate::TensorIndex::Runtime(index)) => {
                    stable &= registers[*index as usize]
                }
                crate::TensorSubscript::Index(crate::TensorIndex::Constant(_)) => {}
            }
        }
        stable &= update
            .condition
            .is_none_or(|index| registers[index as usize]);
        for i in 0..elements {
            stable &= range_stable(
                registers,
                update.value_start + (i * update.value_stride) as crate::Reg,
                *lanes,
            )?;
        }
    }
    for node in nodes {
        if let crate::FoldTensorNode::Select { condition, .. } = node {
            stable &= registers[*condition as usize];
        }
    }
    Some(vec![stable; count * lanes])
}

// SPEC_0021: Exception - dispatch over scalar and tensor dependency transfers.
#[allow(clippy::too_many_lines)]
fn transfer(
    table: &SolvePureCallTable,
    op: &Op,
    r: &mut [bool],
    y: &[bool],
    p: &[bool],
    outputs: &mut Vec<bool>,
) -> Option<()> {
    match op {
        Op::Const { dst, .. } => r[*dst as usize] = true,
        Op::LoadY { dst, index } => r[*dst as usize] = *y.get(*index)?,
        Op::LoadP { dst, index } => r[*dst as usize] = *p.get(*index)?,
        Op::LoadTime { dst } => r[*dst as usize] = false,
        Op::Move { dst, src } => r[*dst as usize] = r[*src as usize],
        Op::Unary { dst, arg, .. } => r[*dst as usize] = r[*arg as usize],
        Op::Binary { dst, lhs, rhs, .. } | Op::Compare { dst, lhs, rhs, .. } => {
            r[*dst as usize] = r[*lhs as usize] && r[*rhs as usize]
        }
        Op::Select {
            dst,
            cond,
            if_true,
            if_false,
        } => r[*dst as usize] = r[*cond as usize] && r[*if_true as usize] && r[*if_false as usize],
        Op::TensorLoad {
            dst_start,
            input,
            input_start,
            count,
            lanes: 1,
            ..
        } => {
            let source = match input {
                TensorInputKind::Y => y,
                TensorInputKind::P => p,
            };
            r.get_mut(*dst_start as usize..*dst_start as usize + count)?
                .copy_from_slice(source.get(*input_start..*input_start + count)?);
        }
        Op::PureCall {
            dst_start,
            input_starts,
            site,
        } => {
            let inputs = input_starts
                .iter()
                .zip(site.inputs())
                .map(|(start, ty)| {
                    r.get(*start as usize..*start as usize + ty.scalar_count() as usize)
                        .map(|v| v.iter().all(|x| *x))
                })
                .collect::<Option<Vec<_>>>()?;
            let owner = table.owner(site.owner())?;
            let values = super::typed_dependencies::outputs(table, owner.body(), &inputs);
            let mut start = *dst_start as usize;
            for (value, output) in values.iter().zip(owner.outputs()) {
                let end = start + output.value_type().scalar_count() as usize;
                r.get_mut(start..end)?.fill(*value);
                start = end;
            }
        }
        Op::TensorBinary {
            dst_start,
            lhs_start,
            rhs_start,
            count,
            lhs_stride,
            rhs_stride,
            lanes: 1,
            ..
        } => {
            for i in 0..*count {
                r[*dst_start as usize + i] = r[*lhs_start as usize + i * lhs_stride]
                    && r[*rhs_start as usize + i * rhs_stride];
            }
        }
        // Every filled element repeats the value lanes.
        Op::TensorFill {
            dst_start,
            value_start,
            count,
            lanes,
        } => {
            let start = *value_start as usize;
            let stable = r.get(start..start + lanes)?.iter().all(|v| *v);
            let start = *dst_start as usize;
            r.get_mut(start..start + count * lanes)?.fill(stable);
        }
        Op::LoadIndexedP {
            dst,
            base,
            count,
            index,
        } => {
            r[*dst as usize] =
                r[*index as usize] && p.get(*base..*base + count)?.iter().all(|v| *v);
        }
        Op::LoadIndexedRegister {
            dst,
            base,
            stride,
            dimensions,
            indices,
        } => {
            r[*dst as usize] = indexed(r, *base as usize, *stride, dimensions, indices, r)?;
        }
        Op::MatrixMultiply {
            dst_start,
            lhs_start,
            rhs_start,
            rows,
            inner,
            columns,
            lanes,
        } => {
            let stable = range_stable(r, *lhs_start, rows * inner * lanes)?
                && range_stable(r, *rhs_start, inner * columns * lanes)?;
            r.get_mut(*dst_start as usize..*dst_start as usize + rows * columns * lanes)?
                .fill(stable);
        }
        Op::TensorTranspose {
            dst_start,
            src_start,
            rows,
            columns,
            element_width,
            lanes,
        } => {
            let count = rows * columns * element_width * lanes;
            let stable = range_stable(r, *src_start, count)?;
            r.get_mut(*dst_start as usize..*dst_start as usize + count)?
                .fill(stable);
        }
        Op::TensorConcatenate {
            dst_start,
            sources,
            lanes,
            ..
        } => {
            let mut stable = true;
            for source in sources {
                let count = source
                    .dimensions
                    .iter()
                    .map(|n| *n as usize)
                    .product::<usize>()
                    * lanes;
                stable &= range_stable(r, source.start, count)?;
            }
            r.get_mut(*dst_start as usize..*dst_start as usize + op.dst_register_count())?
                .fill(stable);
        }
        Op::StoreOutput { src } => outputs.push(r[*src as usize]),
        Op::StoreOutputRange {
            start,
            count,
            stride,
        } => {
            for i in 0..*count {
                outputs.push(*r.get(*start as usize + i * stride)?);
            }
        }
        _ => {
            let dst = op.dst_register()? as usize;
            r.get_mut(dst..dst + op.dst_register_count())?.fill(false);
        }
    }
    Some(())
}

/// Why no dependency order exists.
pub(in crate::fmi) enum OrderError {
    /// A program holds an operation the dependence analysis does not model.
    Unsupported,
    /// A program reads a value nothing settles (time, a state, an input, or a
    /// cycle).
    Unsettled,
}

/// The order in which `candidates` (indices into `programs`) can each be
/// evaluated once from settled values, grouped into dependency levels: a
/// program joins the first level at which every output it computes is settled
/// under `p`, and its `outputs` settle for the levels after it. Returns the
/// order and the number of levels, which is the number of repeated
/// simultaneous sweeps that settle every program from arbitrary values.
pub(in crate::fmi) fn dependency_order(
    table: &SolvePureCallTable,
    programs: &[Vec<Op>],
    candidates: &[usize],
    outputs: &[Vec<usize>],
    y: &[bool],
    p: &mut [bool],
) -> Result<(Vec<usize>, usize), OrderError> {
    let mut pending = candidates.to_vec();
    let mut order = Vec::with_capacity(pending.len());
    let mut levels = 0;
    while !pending.is_empty() {
        let first = order.len();
        let mut waiting = Vec::new();
        for index in pending {
            let settled =
                program_outputs(table, &programs[index], y, p).ok_or(OrderError::Unsupported)?;
            if settled.iter().all(|value| *value) {
                order.push(index);
            } else {
                waiting.push(index);
            }
        }
        if order.len() == first {
            return Err(OrderError::Unsettled);
        }
        for index in &order[first..] {
            outputs[*index].iter().for_each(|target| p[*target] = true);
        }
        levels += 1;
        pending = waiting;
    }
    Ok((order, levels))
}

fn range_stable(values: &[bool], start: crate::Reg, count: usize) -> Option<bool> {
    Some(
        values
            .get(start as usize..start as usize + count)?
            .iter()
            .all(|v| *v),
    )
}

fn indexed(
    values: &[bool],
    base: usize,
    stride: usize,
    dimensions: &[u32],
    indices: &[crate::TensorIndex],
    registers: &[bool],
) -> Option<bool> {
    let count = dimensions
        .iter()
        .try_fold(1usize, |count, extent| count.checked_mul(*extent as usize))?;
    let indices_stable = indices.iter().all(|index| match index {
        crate::TensorIndex::Constant(_) => true,
        crate::TensorIndex::Runtime(register) => {
            registers.get(*register as usize).copied().unwrap_or(false)
        }
    });
    Some(
        indices_stable
            && (0..count).all(|i| values.get(base + i * stride).copied().unwrap_or(false)),
    )
}

fn fold_outputs(
    table: &SolvePureCallTable,
    program: &crate::FunctionFoldProgram,
    registers: &[bool],
    initial: crate::Reg,
    captures: crate::Reg,
    y: &[bool],
    p: &[bool],
) -> Option<Vec<bool>> {
    let carried = registers
        .get(initial as usize..initial as usize + program.carried_count)?
        .to_vec();
    let captures = registers.get(captures as usize..captures as usize + program.capture_count)?;
    fold_frame_outputs(table, program, carried, captures, y, p)
}

fn fold_frame_outputs(
    table: &SolvePureCallTable,
    program: &crate::FunctionFoldProgram,
    mut carried: Vec<bool>,
    captures: &[bool],
    y: &[bool],
    p: &[bool],
) -> Option<Vec<bool>> {
    if program.domain.extents().ok()?.contains(&0) {
        return Some(carried);
    }
    // Conservative meet over a finite Boolean lattice, independent of loop length.
    loop {
        let next = region_outputs(
            table,
            &program.update,
            y,
            p,
            Frame {
                carried: &carried,
                captures,
                conditional: &[],
            },
        )?;
        if next.len() != carried.len() {
            return None;
        }
        let next = carried
            .iter()
            .zip(next)
            .map(|(a, b)| *a && b)
            .collect::<Vec<_>>();
        if next == carried {
            return Some(carried);
        }
        carried = next;
    }
}
