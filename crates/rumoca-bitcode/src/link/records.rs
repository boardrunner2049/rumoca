//! Table-specific relocation; source text and declaration class names stay intact.
use super::{Map, Result, Shift, fields};
use crate::schema::*;

impl Shift for RbcClock {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)?;
        self.provenance.shift(m)?;
        match &mut self.node {
            RbcClockNode::Triggered { condition } => condition.shift(m)?,
            RbcClockNode::Shifted {
                base, condition, ..
            } => {
                base.shift(m)?;
                condition.shift(m)?;
            }
            RbcClockNode::Periodic { .. } => {}
        }
        Ok(())
    }
}

impl Shift for RbcClockOwnership {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.variable.shift(m)?;
        self.clock.shift(m)?;
        self.provenance.shift(m)
    }
}

macro_rules! record {
    ($ty:ident: $($field:ident),+ $(,)?) => {
        impl Shift for $ty {
            fn shift(&mut self, m: &Map<'_>) -> Result<()> {
                fields!(self, m, $($field),+);
                Ok(())
            }
        }
    }
}
record!(RbcExpr: id, value_type, node, provenance);
record!(RbcRelation: id, expression, provenance);
record!(RbcCondition: id, node, provenance);
record!(RbcRoot: id, relation, activation, provenance);
record!(RbcEventAction: id, trigger, guard, action, provenance);
record!(RbcInitialDiscreteValue: target, value, provenance);
record!(RbcDiscreteDefinition: targets, branches, provenance);
record!(RbcModelEventTransaction: targets, steps, provenance);
record!(RbcModelEventStep: trigger, guard, clock, definitions, provenance);
record!(RbcModelEventDefinition: target, value, provenance);
record!(RbcPreviousValue: variable, clock, provenance);
record!(RbcTerminal: provenance);
record!(RbcStructuredRoot: domain, expression, provenance);
record!(RbcDelay: source, delay, provenance);
record!(RbcPositiveParameter: expression, provenance);
impl Shift for RbcDelayKind {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Parameter { delay_time } => delay_time.shift(m),
            Self::Bounded {
                delay_time,
                maximum,
            } => {
                delay_time.shift(m)?;
                maximum.shift(m)
            }
        }
    }
}

impl Shift for RbcProvenance {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.span.source.shift(m)
    }
}
impl Shift for RbcSource {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)
    }
}
impl Shift for RbcType {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)?;
        if let Some(record) = &mut self.record {
            m.qualify(&mut record.name);
            for field in &mut record.fields {
                field.value_type.shift(m)?;
            }
        }
        Ok(())
    }
}
impl Shift for RbcVariable {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(
            self,
            m,
            id,
            value_type,
            declaration,
            component,
            binding,
            start,
            min,
            max,
            nominal
        );
        if let Some(contract) = &mut self.contract {
            contract.binding_depends_on.shift(m)?;
        }
        m.qualify(&mut self.name);
        Ok(())
    }
}
impl Shift for RbcComponent {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)?;
        m.qualify(&mut self.path);
        Ok(())
    }
}
impl Shift for RbcDomain {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, id, parent, provenance);
        Ok(())
    }
}
impl Shift for RbcFunction {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(
            self,
            m,
            id,
            results,
            declaration,
            calls,
            values,
            folds,
            body
        );
        for derivative in &mut self.derivatives {
            derivative.target.shift(m)?;
            derivative.provenance.shift(m)?;
            if let Some((source, _)) = &mut derivative.previous {
                source.shift(m)?;
            }
        }
        for p in &mut self.parameters {
            p.value_type.shift(m)?;
            p.declaration.shift(m)?;
        }
        m.qualify(&mut self.name);
        Ok(())
    }
}

// A carried body names arena expressions, types, domains and sources like any
// other table. Owner-local ordinals -- values, folds, definitions -- are
// addresses *within* the function and never move.
impl Shift for RbcFunctionValue {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, value_type, declaration);
        Ok(())
    }
}
impl Shift for RbcFunctionDefinition {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.expression.shift(m)
    }
}
impl Shift for RbcFunctionFold {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(
            self, m, domain, parameters, initial, update, output, provenance
        );
        Ok(())
    }
}
impl Shift for RbcFunctionConditional {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, conditions, branches, fallback);
        Ok(())
    }
}
impl Shift for RbcFunctionStatement {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Assignment {
                expression,
                provenance,
                ..
            } => {
                expression.shift(m)?;
                provenance.shift(m)
            }
            Self::Assertion {
                condition,
                message,
                provenance,
                ..
            } => {
                condition.shift(m)?;
                message.shift(m)?;
                provenance.shift(m)
            }
            Self::AssignmentGroup {
                conditional,
                expressions,
                provenance,
                ..
            } => {
                conditional.shift(m)?;
                expressions.shift(m)?;
                provenance.shift(m)
            }
            Self::For {
                statements,
                provenance,
                ..
            } => {
                statements.shift(m)?;
                provenance.shift(m)
            }
        }
    }
}
impl Shift for RbcExternalArgument {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Input { expression } => expression.shift(m),
            Self::Output { .. } => Ok(()),
        }
    }
}
impl Shift for RbcFunctionBody {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::ElidedModelica => Ok(()),
            Self::Modelica { statements } => statements.shift(m),
            Self::External { arguments, .. } => arguments.shift(m),
        }
    }
}
impl Map<'_> {
    pub(super) fn equation_body(&self, eq: &mut RbcEquation) -> Result<()> {
        fields!(
            eq,
            self,
            residual,
            provenance,
            reads,
            reads_derivative,
            reads_previous
        );
        Ok(())
    }
    pub(super) fn family_body(&self, family: &mut RbcEquationFamily) -> Result<()> {
        fields!(
            family,
            self,
            domain,
            bodies,
            reads,
            reads_derivative,
            reads_previous,
            provenance
        );
        Ok(())
    }
}
impl Shift for RbcEquation {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)?;
        m.equation_body(self)
    }
}
impl Shift for RbcEquationFamily {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        self.id.shift(m)?;
        m.family_body(self)
    }
}
impl Shift for RbcDiscreteRealEquation {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        m.discrete_real_equations.shift(&mut self.id.0)?;
        fields!(
            self,
            m,
            residual,
            reads,
            reads_derivative,
            reads_previous,
            provenance
        );
        if let RbcDiscreteRealActivation::When { trigger, guard } = &mut self.activation {
            trigger.shift(m)?;
            guard.shift(m)?;
        }
        Ok(())
    }
}
impl Shift for RbcDiscreteBranch {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, values, provenance);
        if let RbcDiscreteActivation::When { trigger, guard } = &mut self.activation {
            trigger.shift(m)?;
            guard.shift(m)?;
        }
        Ok(())
    }
}
impl Shift for RbcTimeEvent {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        m.time_events.shift(&mut self.id.0)?;
        self.provenance.shift(m)?;
        if let RbcSchedule::Dynamic { deadline } = &mut self.schedule {
            deadline.shift(m)?;
        }
        Ok(())
    }
}
impl Shift for RbcConnection {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, id, left, right, equation, provenance);
        m.qualify(&mut self.left_connector);
        m.qualify(&mut self.right_connector);
        Ok(())
    }
}
impl Shift for RbcConnectionSet {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, id, potentials, potential_equations, provenance);
        for connector in &mut self.connectors {
            m.qualify(connector);
        }
        for balance in &mut self.balances {
            balance.equation.shift(m)?;
            for term in &mut balance.terms {
                term.variable.shift(m)?;
            }
        }
        Ok(())
    }
}
impl Shift for RbcTracePoint {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        fields!(self, m, id, variable, connection, connection_set);
        m.qualify(&mut self.label);
        Ok(())
    }
}
impl Shift for RbcConnectorType {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        m.connector_types.shift(&mut self.id)?;
        m.qualify(&mut self.name);
        Ok(())
    }
}
impl Shift for RbcConnectorInstance {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        m.connectors.shift(&mut self.id)?;
        m.connector_types.shift(&mut self.type_id)?;
        fields!(self, m, owner, provenance);
        for member in &mut self.members {
            member.variable.shift(m)?;
        }
        m.qualify(&mut self.path);
        Ok(())
    }
}
