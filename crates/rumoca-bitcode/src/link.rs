//! Namespaced, disjoint linking of public equation artifacts.
//!
//! This is an interchange operation, not DAE structural lowering (SPEC_0007).
//! It preserves equations, boundaries and provenance; it never guesses wiring.

mod expressions;
mod records;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::schema::*;
use crate::validate::{ValidateOptions, recompute_summary, validate};

/// One module instance. The same artifact may be linked under different names.
pub struct LinkInput<'a> {
    pub namespace: &'a str,
    pub file: &'a RbcFile,
}

#[derive(Debug, thiserror::Error)]
#[error("bitcode link: {0}")]
pub struct LinkError(pub String);

type Result<T> = std::result::Result<T, LinkError>;

/// Link independent modules without mutating inputs or inferring connections.
///
/// Namespaces must be distinct simple identifiers. Execution projections are
/// rejected unless `discard_execution` explicitly authorizes their removal,
/// including numerical edits and instrumentation. The result must be lowered
/// again before execution. A successful link proves structural consistency,
/// not solvability or runtime support.
pub fn link(name: &str, inputs: &[LinkInput<'_>], discard_execution: bool) -> Result<RbcFile> {
    if name.trim().is_empty() || inputs.is_empty() {
        return Err(LinkError(
            "a model name and at least one input are required".into(),
        ));
    }
    let mut names = BTreeSet::new();
    let mut model = crate::build::Builder::new(name).finish();
    // Builder seeds a source and types; a link uses only the input tables.
    model.sources.clear();
    model.types.clear();
    for input in inputs {
        check_input(input, discard_execution, &mut names)?;
        let map = Map::new(&model, &input.file.model, input.namespace)?;
        let mut part = input.file.model.clone();
        map.relocate(&mut part)?;
        append(&mut model, part);
    }
    // Structured rows may vastly outnumber their compact table entries.
    model
        .equation_families
        .iter()
        .try_fold(0u32, |total, family| {
            total
                .checked_add(family.scalar_rows)
                .ok_or_else(|| LinkError("family scalar-row count overflow".into()))
        })?;
    recompute_summary(&mut model);
    check_model(&model)?;
    Ok(RbcFile {
        magic: RBC_MAGIC.into(),
        bitcode_version: RBC_VERSION,
        producer: format!("rumoca-bitcode-link {}", env!("CARGO_PKG_VERSION")),
        execution: None,
        model,
    })
}

fn check_model(model: &RbcModel) -> Result<()> {
    let options = ValidateOptions {
        reject_unsupported: true,
    };
    validate(model, &options).map_err(|errors| {
        LinkError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}

fn check_input(input: &LinkInput<'_>, discard: bool, names: &mut BTreeSet<String>) -> Result<()> {
    let ns = input.namespace;
    let valid = !ns.is_empty()
        && ns
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if !valid || !names.insert(ns.to_owned()) {
        return Err(LinkError(format!(
            "namespace {ns:?} must be a unique simple identifier"
        )));
    }
    input.file.check_header().map_err(LinkError)?;
    if input.file.execution.is_some() && !discard {
        return Err(LinkError(format!(
            "{ns}: executable input; explicitly discard execution to link its equations (numerical edits and instrumentation will be lost)"
        )));
    }
    check_model(&input.file.model).map_err(|e| LinkError(format!("{ns}: {e}")))
}

/// How one table's ids move.
///
/// Linking offsets every table. A pass that deletes entries renumbers the
/// survivors through a map, and one that needs to know what is referenced
/// records ids without moving them. All three walk the same exhaustive
/// visitor, so a table or a reference added to the schema is handled by
/// every one of them or by none.
#[derive(Clone, Copy)]
enum Range<'a> {
    Offset {
        start: u32,
        len: u32,
        table: &'static str,
    },
    /// Old id to new id; `None` for an entry the rewrite removed.
    Remap {
        map: &'a [Option<u32>],
        table: &'static str,
    },
    /// Mark every id seen and leave it where it is.
    Collect {
        seen: &'a std::cell::RefCell<Vec<bool>>,
        table: &'static str,
    },
}

impl Range<'_> {
    fn new(start: usize, len: usize, table: &'static str) -> Result<Self> {
        let start = u32::try_from(start).map_err(|_| LinkError(format!("{table}: ID overflow")))?;
        let len = u32::try_from(len).map_err(|_| LinkError(format!("{table}: ID overflow")))?;
        start
            .checked_add(len)
            .ok_or_else(|| LinkError(format!("{table}: ID overflow")))?;
        Ok(Self::Offset { start, len, table })
    }

    fn shift(self, id: &mut u32) -> Result<()> {
        let out_of_bounds = |table: &str, len: usize| {
            LinkError(format!("{table} reference {id} out of bounds ({len})"))
        };
        match self {
            Self::Offset { start, len, table } => {
                if *id >= len {
                    return Err(out_of_bounds(table, len as usize));
                }
                *id += start;
            }
            Self::Remap { map, table } => {
                let moved = map
                    .get(*id as usize)
                    .ok_or_else(|| out_of_bounds(table, map.len()))?;
                *id = moved.ok_or_else(|| {
                    LinkError(format!("{table} {id} was removed but is still referenced"))
                })?;
            }
            Self::Collect { seen, table } => {
                let mut seen = seen.borrow_mut();
                let len = seen.len();
                *seen
                    .get_mut(*id as usize)
                    .ok_or_else(|| out_of_bounds(table, len))? = true;
            }
        }
        Ok(())
    }
}

// A single table catalog supplies relocation ranges and concatenation. IDs in
// initial/discrete equations, time events and initial families have their own
// spaces even though the schema reuses their Rust newtype.
macro_rules! tables {
    ($($field:ident),+ $(,)?) => {
        struct Map<'a> { namespace: Option<&'a str>, $($field: Range<'a>,)+ }
        impl<'a> Map<'a> {
            fn new(out: &RbcModel, input: &RbcModel, namespace: &'a str) -> Result<Self> {
                Ok(Self { namespace: Some(namespace), $($field: Range::new(out.$field.len(), input.$field.len(), stringify!($field))?,)+ })
            }
            /// Every table left where it is, names unqualified.
            fn identity(model: &RbcModel) -> Result<Self> {
                Ok(Self { namespace: None, $($field: Range::new(0, model.$field.len(), stringify!($field))?,)+ })
            }
        }
        fn append(out: &mut RbcModel, input: RbcModel) {
            // Exhaustive destructuring makes adding a table a linker review.
            let RbcModel { name: _, summary: _, $($field,)+ } = input;
            $(out.$field.extend($field);)+
        }
    }
}
tables!(
    sources,
    types,
    variables,
    expressions,
    equations,
    initial_equations,
    domains,
    discrete_real_equations,
    initial_discrete_values,
    initial_parameter_values,
    functions,
    equation_families,
    initial_equation_families,
    relations,
    conditions,
    clocks,
    clock_ownerships,
    roots,
    events,
    time_events,
    connections,
    connection_sets,
    components,
    trace_points,
    discrete_definitions,
    model_event_transactions,
    previous_values,
    terminals,
    structured_roots,
    delays,
    connector_types,
    connectors
);

/// Every expression referenced from outside the expression arena: equation
/// residuals, attributes, conditions, function bodies, events, and so on.
///
/// Operands are deliberately not walked; a caller closes over them with
/// [`crate::build::references`]. Visiting is by the same exhaustive walk
/// linking uses, so no table can be forgotten here without being forgotten
/// by the linker too.
pub(crate) fn expression_roots(model: &RbcModel) -> Result<Vec<bool>> {
    let seen = std::cell::RefCell::new(vec![false; model.expressions.len()]);
    let mut outside = model.clone();
    outside.expressions.clear();
    let mut map = Map::identity(model)?;
    map.expressions = Range::Collect {
        seen: &seen,
        table: "expressions",
    };
    map.relocate(&mut outside)?;
    Ok(seen.into_inner())
}

/// Drop removed expressions and functions and renumber every reference to
/// the survivors. `expressions` and `functions` map old ids to new; `None`
/// removes the entry, and a remaining reference to one is an error.
pub(crate) fn renumber(
    model: &mut RbcModel,
    expressions: &[Option<u32>],
    functions: &[Option<u32>],
) -> Result<()> {
    let mut index = 0;
    model.expressions.retain(|_| {
        index += 1;
        expressions.get(index - 1).copied().flatten().is_some()
    });
    let mut index = 0;
    model.functions.retain(|_| {
        index += 1;
        functions.get(index - 1).copied().flatten().is_some()
    });
    let mut map = Map::identity(model)?;
    map.expressions = Range::Remap {
        map: expressions,
        table: "expressions",
    };
    map.functions = Range::Remap {
        map: functions,
        table: "functions",
    };
    map.relocate(model)
}

trait Shift {
    fn shift(&mut self, map: &Map<'_>) -> Result<()>;
}
impl<T: Shift> Shift for Vec<T> {
    fn shift(&mut self, map: &Map<'_>) -> Result<()> {
        self.iter_mut().try_for_each(|v| v.shift(map))
    }
}
impl<T: Shift> Shift for Option<T> {
    fn shift(&mut self, map: &Map<'_>) -> Result<()> {
        if let Some(v) = self {
            v.shift(map)?;
        }
        Ok(())
    }
}
macro_rules! ids {
    ($($ty:ident => $table:ident),+ $(,)?) => {$(
        impl Shift for $ty {
            fn shift(&mut self, map: &Map<'_>) -> Result<()> { map.$table.shift(&mut self.0) }
        }
    )+}
}
ids!(SourceId=>sources, TypeId=>types, VariableId=>variables, ExprId=>expressions,
    EquationId=>equations, RelationId=>relations, ConditionId=>conditions,
    RootId=>roots, EventId=>events, ConnectionId=>connections,
    ConnectionSetId=>connection_sets, ComponentId=>components,
    TracePointId=>trace_points, DomainId=>domains, FunctionId=>functions,
    FamilyId=>equation_families, ClockId=>clocks, PreviousId=>previous_values,
    TerminalId=>terminals, DelayId=>delays);

macro_rules! fields {
    ($value:ident, $map:ident, $($field:ident),+ $(,)?) => { $( $value.$field.shift($map)?; )+ }
}
use fields;

impl Map<'_> {
    fn qualify(&self, value: &mut String) {
        let Some(namespace) = self.namespace else {
            return;
        };
        *value = if value.is_empty() {
            namespace.to_owned()
        } else {
            format!("{namespace}.{value}")
        };
    }

    fn relocate(&self, m: &mut RbcModel) -> Result<()> {
        m.sources.shift(self)?;
        m.types.shift(self)?;
        m.variables.shift(self)?;
        m.expressions.shift(self)?;
        m.equations.shift(self)?;
        for eq in &mut m.initial_equations {
            self.initial_equations.shift(&mut eq.id.0)?;
            self.equation_body(eq)?;
        }
        m.domains.shift(self)?;
        m.functions.shift(self)?;
        m.equation_families.shift(self)?;
        for family in &mut m.initial_equation_families {
            self.initial_equation_families.shift(&mut family.id.0)?;
            self.family_body(family)?;
        }
        m.discrete_real_equations.shift(self)?;
        m.initial_discrete_values.shift(self)?;
        m.initial_parameter_values.shift(self)?;
        m.discrete_definitions.shift(self)?;
        m.model_event_transactions.shift(self)?;
        m.previous_values.shift(self)?;
        m.terminals.shift(self)?;
        m.structured_roots.shift(self)?;
        m.delays.shift(self)?;
        m.relations.shift(self)?;
        m.conditions.shift(self)?;
        m.clocks.shift(self)?;
        m.clock_ownerships.shift(self)?;
        m.roots.shift(self)?;
        m.events.shift(self)?;
        m.time_events.shift(self)?;
        m.components.shift(self)?;
        m.connections.shift(self)?;
        m.connection_sets.shift(self)?;
        m.trace_points.shift(self)?;
        m.connector_types.shift(self)?;
        m.connectors.shift(self)?;
        // These tables have no entry IDs, but are still concatenated/count-checked.
        let _ = (
            self.initial_discrete_values,
            self.initial_parameter_values,
            self.discrete_definitions,
            self.model_event_transactions,
            self.structured_roots,
            self.clock_ownerships,
        );
        Ok(())
    }
}
