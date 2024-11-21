mod monomial;

use std::{
    cmp::max,
    fmt::{Display, Debug},
    mem::MaybeUninit,
    ops::{Add, Deref, Mul, Neg, Sub},
};

use ff::Field;
use ff_ext::ExtensionField;
use goldilocks::SmallField;
use std::collections::VecDeque;

#[cfg(test)]
use multilinear_extensions::virtual_poly_v2::ArcMultilinearExtension;

use crate::{
    circuit_builder::CircuitBuilder,
    error::ZKVMError,
    structs::{ChallengeId, WitnessId},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expression<E: ExtensionField> {
    /// WitIn(Id)
    WitIn(WitnessId),
    /// Fixed
    Fixed(Fixed),
    /// Constant poly
    Constant(E::BaseField),
    /// This is the sum of two expression
    Sum(Box<Expression<E>>, Box<Expression<E>>),
    /// This is the product of two polynomials
    Product(Box<Expression<E>>, Box<Expression<E>>),
    /// This is x, a, b expr to represent ax + b polynomial
    ScaledSum(Box<Expression<E>>, Box<Expression<E>>, Box<Expression<E>>),
    Challenge(ChallengeId, usize, E, E), // (challenge_id, power, scalar, offset)
}

fn ext_field_as_limbs_no_trait<T: Debug>(scalar: &T) -> [String; 2] {
    let scalar_str = format!("{:?}", scalar);
    let str_seg: Vec<&str> = scalar_str.split(&['(', ')']).collect();
    [str_seg[2].to_string(), str_seg[4].to_string()]
}

impl<E: ExtensionField> Expression<E> {
    pub fn expr_as_list(&self) -> (usize, Vec<String>) {
        // Print out the expression as a list (unroll all recursion) of form:
        #[derive(Debug)]
        struct ExpressionEntry<E: ExtensionField> {
            expr_ty: u8,
            id: u16, // wit_id or challenge_id
            fixed: usize, // fixed or power
            constant: E::BaseField,
            scalar: E,
            offset: E,
            l0: usize, // limbs point to the indices of subexpressions
            l1: usize,
            l2: usize,
        }

        let mut sp = 0;
        let mut expr_list: Vec<ExpressionEntry<E>> = Vec::new();
        let mut queue: VecDeque<Expression<E>> = VecDeque::from([self.clone()]);
        // Assume no recursion, etc.
        while queue.len() > 0 {
            let next_expr = queue.pop_front().unwrap();
            sp += 1;
            expr_list.push(match next_expr {
                Expression::WitIn(wit_id) => ExpressionEntry {
                    expr_ty: 0,
                    id: wit_id,
                    fixed: 0,
                    constant: E::BaseField::from(0),
                    scalar: E::from(0),
                    offset: E::from(0),
                    l0: 0,
                    l1: 0,
                    l2: 0,
                },
                Expression::Fixed(fixed) => ExpressionEntry {
                    expr_ty: 1,
                    id: 0,
                    fixed: fixed.0,
                    constant: E::BaseField::from(0),
                    scalar: E::from(0),
                    offset: E::from(0),
                    l0: 0,
                    l1: 0,
                    l2: 0,
                },
                Expression::Constant(cnst) => ExpressionEntry {
                    expr_ty: 2,
                    id: 0,
                    fixed: 0,
                    constant: cnst.clone(),
                    scalar: E::from(0),
                    offset: E::from(0),
                    l0: 0,
                    l1: 0,
                    l2: 0,
                },
                Expression::Sum(a, b) => {
                    // Allocated entry would be indexed sp + queue.len()
                    queue.push_back(*a);
                    queue.push_back(*b);
                    ExpressionEntry {
                        expr_ty: 3,
                        id: 0,
                        fixed: 0,
                        constant: E::BaseField::from(0),
                        scalar: E::from(0),
                        offset: E::from(0),
                        l0: sp + queue.len() - 2,
                        l1: sp + queue.len() - 1,
                        l2: 0,
                    }
                },
                Expression::Product(a, b) => {
                    // Allocated entry would be indexed sp + queue.len()
                    queue.push_back(*a);
                    queue.push_back(*b);
                    ExpressionEntry {
                        expr_ty: 4,
                        id: 0,
                        fixed: 0,
                        constant: E::BaseField::from(0),
                        scalar: E::from(0),
                        offset: E::from(0),
                        l0: sp + queue.len() - 2,
                        l1: sp + queue.len() - 1,
                        l2: 0,
                    }
                },
                Expression::ScaledSum(a, b, c) => {
                    // Allocated entry would be indexed sp + queue.len()
                    queue.push_back(*a);
                    queue.push_back(*b);
                    queue.push_back(*c);
                    ExpressionEntry {
                        expr_ty: 5,
                        id: 0,
                        fixed: 0,
                        constant: E::BaseField::from(0),
                        scalar: E::from(0),
                        offset: E::from(0),
                        l0: sp + queue.len() - 3,
                        l1: sp + queue.len() - 2,
                        l2: sp + queue.len() - 1,
                    }
                },
                Expression::Challenge(challenge_id, power, scalar, offset) => ExpressionEntry {
                    expr_ty: 6,
                    id: challenge_id,
                    fixed: power,
                    constant: E::BaseField::from(0),
                    scalar: scalar.clone(),
                    offset: offset.clone(),
                    l0: 0,
                    l1: 0,
                    l2: 0,
                },
            });
        }
        let expr_len = expr_list.len();
        let mut expr_entries = Vec::new();

        for expr in expr_list {
            expr_entries.extend([expr.expr_ty.to_string(), expr.id.to_string(), expr.fixed.to_string()]);
            let const_str = format!("{:?}", expr.constant);
            let str_seg: Vec<&str> = const_str.split(&['(', ')']).collect();
            expr_entries.push(str_seg[1].to_string());
            expr_entries.extend(ext_field_as_limbs_no_trait(&expr.scalar));
            expr_entries.extend(ext_field_as_limbs_no_trait(&expr.offset));
            expr_entries.extend([expr.l0.to_string(), expr.l1.to_string(), expr.l2.to_string()]);
        }
        (expr_len, expr_entries)
    }
}

/// this is used as finite state machine state
/// for differentiate an expression is in monomial form or not
enum MonomialState {
    SumTerm,
    ProductTerm,
}

impl<E: ExtensionField> Expression<E> {
    pub const ZERO: Expression<E> = Expression::Constant(E::BaseField::ZERO);
    pub const ONE: Expression<E> = Expression::Constant(E::BaseField::ONE);

    pub fn degree(&self) -> usize {
        match self {
            Expression::Fixed(_) => 1,
            Expression::WitIn(_) => 1,
            Expression::Constant(_) => 0,
            Expression::Sum(a_expr, b_expr) => max(a_expr.degree(), b_expr.degree()),
            Expression::Product(a_expr, b_expr) => a_expr.degree() + b_expr.degree(),
            Expression::ScaledSum(_, _, _) => 1,
            Expression::Challenge(_, _, _, _) => 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn evaluate<T>(
        &self,
        fixed_in: &impl Fn(&Fixed) -> T,
        wit_in: &impl Fn(WitnessId) -> T, // witin id
        constant: &impl Fn(E::BaseField) -> T,
        challenge: &impl Fn(ChallengeId, usize, E, E) -> T,
        sum: &impl Fn(T, T) -> T,
        product: &impl Fn(T, T) -> T,
        scaled: &impl Fn(T, T, T) -> T,
    ) -> T {
        match self {
            Expression::Fixed(f) => fixed_in(f),
            Expression::WitIn(witness_id) => wit_in(*witness_id),
            Expression::Constant(scalar) => constant(*scalar),
            Expression::Sum(a, b) => {
                let a = a.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                let b = b.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                sum(a, b)
            }
            Expression::Product(a, b) => {
                let a = a.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                let b = b.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                product(a, b)
            }
            Expression::ScaledSum(x, a, b) => {
                let x = x.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                let a = a.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                let b = b.evaluate(fixed_in, wit_in, constant, challenge, sum, product, scaled);
                scaled(x, a, b)
            }
            Expression::Challenge(challenge_id, pow, scalar, offset) => {
                challenge(*challenge_id, *pow, *scalar, *offset)
            }
        }
    }

    pub fn is_monomial_form(&self) -> bool {
        Self::is_monomial_form_inner(MonomialState::SumTerm, self)
    }

    pub fn to_monomial_form(&self) -> Self {
        self.to_monomial_form_inner()
    }

    pub fn unpack_sum(&self) -> Option<(Expression<E>, Expression<E>)> {
        match self {
            Expression::Sum(a, b) => Some((a.deref().clone(), b.deref().clone())),
            _ => None,
        }
    }

    fn is_zero_expr(expr: &Expression<E>) -> bool {
        match expr {
            Expression::Fixed(_) => false,
            Expression::WitIn(_) => false,
            Expression::Constant(c) => *c == E::BaseField::ZERO,
            Expression::Sum(a, b) => Self::is_zero_expr(a) && Self::is_zero_expr(b),
            Expression::Product(a, b) => Self::is_zero_expr(a) || Self::is_zero_expr(b),
            Expression::ScaledSum(x, a, b) => {
                (Self::is_zero_expr(x) || Self::is_zero_expr(a)) && Self::is_zero_expr(b)
            }
            Expression::Challenge(_, _, scalar, offset) => *scalar == E::ZERO && *offset == E::ZERO,
        }
    }

    fn is_monomial_form_inner(s: MonomialState, expr: &Expression<E>) -> bool {
        match (expr, s) {
            (
                Expression::Fixed(_)
                | Expression::WitIn(_)
                | Expression::Challenge(..)
                | Expression::Constant(_),
                _,
            ) => true,
            (Expression::Sum(a, b), MonomialState::SumTerm) => {
                Self::is_monomial_form_inner(MonomialState::SumTerm, a)
                    && Self::is_monomial_form_inner(MonomialState::SumTerm, b)
            }
            (Expression::Sum(_, _), MonomialState::ProductTerm) => false,
            (Expression::Product(a, b), MonomialState::SumTerm) => {
                Self::is_monomial_form_inner(MonomialState::ProductTerm, a)
                    && Self::is_monomial_form_inner(MonomialState::ProductTerm, b)
            }
            (Expression::Product(a, b), MonomialState::ProductTerm) => {
                Self::is_monomial_form_inner(MonomialState::ProductTerm, a)
                    && Self::is_monomial_form_inner(MonomialState::ProductTerm, b)
            }
            (Expression::ScaledSum(_, _, _), MonomialState::SumTerm) => true,
            (Expression::ScaledSum(x, a, b), MonomialState::ProductTerm) => {
                Self::is_zero_expr(x) || Self::is_zero_expr(a) || Self::is_zero_expr(b)
            }
        }
    }
}

impl<E: ExtensionField> Neg for Expression<E> {
    type Output = Expression<E>;
    fn neg(self) -> Self::Output {
        match self {
            Expression::Fixed(_) | Expression::WitIn(_) => Expression::ScaledSum(
                Box::new(self),
                Box::new(Expression::Constant(E::BaseField::ONE.neg())),
                Box::new(Expression::Constant(E::BaseField::ZERO)),
            ),
            Expression::Constant(c1) => Expression::Constant(c1.neg()),
            Expression::Sum(a, b) => {
                Expression::Sum(Box::new(-a.deref().clone()), Box::new(-b.deref().clone()))
            }
            Expression::Product(a, b) => {
                Expression::Product(Box::new(-a.deref().clone()), Box::new(b.deref().clone()))
            }
            Expression::ScaledSum(x, a, b) => Expression::ScaledSum(
                x,
                Box::new(-a.deref().clone()),
                Box::new(-b.deref().clone()),
            ),
            Expression::Challenge(challenge_id, pow, scalar, offset) => {
                Expression::Challenge(challenge_id, pow, scalar.neg(), offset.neg())
            }
        }
    }
}

impl<E: ExtensionField> Add for Expression<E> {
    type Output = Expression<E>;
    fn add(self, rhs: Expression<E>) -> Expression<E> {
        match (&self, &rhs) {
            // constant + challenge
            (
                Expression::Constant(c1),
                Expression::Challenge(challenge_id, pow, scalar, offset),
            )
            | (
                Expression::Challenge(challenge_id, pow, scalar, offset),
                Expression::Constant(c1),
            ) => Expression::Challenge(*challenge_id, *pow, *scalar, *offset + c1),

            // challenge + challenge
            (
                Expression::Challenge(challenge_id1, pow1, scalar1, offset1),
                Expression::Challenge(challenge_id2, pow2, scalar2, offset2),
            ) => {
                if challenge_id1 == challenge_id2 && pow1 == pow2 {
                    Expression::Challenge(
                        *challenge_id1,
                        *pow1,
                        *scalar1 + scalar2,
                        *offset1 + offset2,
                    )
                } else {
                    Expression::Sum(Box::new(self), Box::new(rhs))
                }
            }

            // constant + constant
            (Expression::Constant(c1), Expression::Constant(c2)) => Expression::Constant(*c1 + c2),

            // constant + scaledsum
            (c1 @ Expression::Constant(_), Expression::ScaledSum(x, a, b))
            | (Expression::ScaledSum(x, a, b), c1 @ Expression::Constant(_)) => {
                Expression::ScaledSum(
                    x.clone(),
                    a.clone(),
                    Box::new(b.deref().clone() + c1.clone()),
                )
            }

            // challenge + scaledsum
            (c1 @ Expression::Challenge(..), Expression::ScaledSum(x, a, b))
            | (Expression::ScaledSum(x, a, b), c1 @ Expression::Challenge(..)) => {
                Expression::ScaledSum(
                    x.clone(),
                    a.clone(),
                    Box::new(b.deref().clone() + c1.clone()),
                )
            }

            _ => Expression::Sum(Box::new(self), Box::new(rhs)),
        }
    }
}

impl<E: ExtensionField> Sub for Expression<E> {
    type Output = Expression<E>;
    fn sub(self, rhs: Expression<E>) -> Expression<E> {
        match (&self, &rhs) {
            // constant - challenge
            (
                Expression::Constant(c1),
                Expression::Challenge(challenge_id, pow, scalar, offset),
            ) => Expression::Challenge(*challenge_id, *pow, *scalar, offset.neg() + c1),

            // challenge - constant
            (
                Expression::Challenge(challenge_id, pow, scalar, offset),
                Expression::Constant(c1),
            ) => Expression::Challenge(*challenge_id, *pow, *scalar, *offset - c1),

            // challenge - challenge
            (
                Expression::Challenge(challenge_id1, pow1, scalar1, offset1),
                Expression::Challenge(challenge_id2, pow2, scalar2, offset2),
            ) => {
                if challenge_id1 == challenge_id2 && pow1 == pow2 {
                    Expression::Challenge(
                        *challenge_id1,
                        *pow1,
                        *scalar1 - scalar2,
                        *offset1 - offset2,
                    )
                } else {
                    Expression::Sum(Box::new(self), Box::new(-rhs))
                }
            }

            // constant - constant
            (Expression::Constant(c1), Expression::Constant(c2)) => Expression::Constant(*c1 - c2),

            // constant - scalesum
            (c1 @ Expression::Constant(_), Expression::ScaledSum(x, a, b)) => {
                Expression::ScaledSum(
                    x.clone(),
                    Box::new(-a.deref().clone()),
                    Box::new(c1.clone() - b.deref().clone()),
                )
            }

            // scalesum - constant
            (Expression::ScaledSum(x, a, b), c1 @ Expression::Constant(_)) => {
                Expression::ScaledSum(
                    x.clone(),
                    a.clone(),
                    Box::new(b.deref().clone() - c1.clone()),
                )
            }

            // challenge - scalesum
            (c1 @ Expression::Challenge(..), Expression::ScaledSum(x, a, b)) => {
                Expression::ScaledSum(
                    x.clone(),
                    Box::new(-a.deref().clone()),
                    Box::new(c1.clone() - b.deref().clone()),
                )
            }

            // scalesum - challenge
            (Expression::ScaledSum(x, a, b), c1 @ Expression::Challenge(..)) => {
                Expression::ScaledSum(
                    x.clone(),
                    a.clone(),
                    Box::new(b.deref().clone() - c1.clone()),
                )
            }

            _ => Expression::Sum(Box::new(self), Box::new(-rhs)),
        }
    }
}

impl<E: ExtensionField> Mul for Expression<E> {
    type Output = Expression<E>;
    fn mul(self, rhs: Expression<E>) -> Expression<E> {
        match (&self, &rhs) {
            // constant * witin
            (c @ Expression::Constant(_), w @ Expression::WitIn(..))
            | (w @ Expression::WitIn(..), c @ Expression::Constant(_)) => Expression::ScaledSum(
                Box::new(w.clone()),
                Box::new(c.clone()),
                Box::new(Expression::Constant(E::BaseField::ZERO)),
            ),
            // challenge * witin
            (c @ Expression::Challenge(..), w @ Expression::WitIn(..))
            | (w @ Expression::WitIn(..), c @ Expression::Challenge(..)) => Expression::ScaledSum(
                Box::new(w.clone()),
                Box::new(c.clone()),
                Box::new(Expression::Constant(E::BaseField::ZERO)),
            ),
            // constant * challenge
            (
                Expression::Constant(c1),
                Expression::Challenge(challenge_id, pow, scalar, offset),
            )
            | (
                Expression::Challenge(challenge_id, pow, scalar, offset),
                Expression::Constant(c1),
            ) => Expression::Challenge(*challenge_id, *pow, *scalar * c1, *offset * c1),
            // challenge * challenge
            (
                Expression::Challenge(challenge_id1, pow1, s1, offset1),
                Expression::Challenge(challenge_id2, pow2, s2, offset2),
            ) => {
                if challenge_id1 == challenge_id2 {
                    // (s1 * s2 * c1^(pow1 + pow2) + offset2 * s1 * c1^(pow1) + offset1 * s2 * c2^(pow2))
                    // + offset1 * offset2

                    // (s1 * s2 * c1^(pow1 + pow2) + offset1 * offset2
                    let mut result = Expression::Challenge(
                        *challenge_id1,
                        pow1 + pow2,
                        *s1 * s2,
                        *offset1 * offset2,
                    );

                    // offset2 * s1 * c1^(pow1)
                    if *s1 != E::ZERO && *offset2 != E::ZERO {
                        result = Expression::Sum(
                            Box::new(result),
                            Box::new(Expression::Challenge(
                                *challenge_id1,
                                *pow1,
                                *offset2 * *s1,
                                E::ZERO,
                            )),
                        );
                    }

                    // offset1 * s2 * c2^(pow2))
                    if *s2 != E::ZERO && *offset1 != E::ZERO {
                        result = Expression::Sum(
                            Box::new(result),
                            Box::new(Expression::Challenge(
                                *challenge_id1,
                                *pow2,
                                *offset1 * *s2,
                                E::ZERO,
                            )),
                        );
                    }

                    result
                } else {
                    Expression::Product(Box::new(self), Box::new(rhs))
                }
            }

            // constant * constant
            (Expression::Constant(c1), Expression::Constant(c2)) => Expression::Constant(*c1 * c2),
            // scaledsum * constant
            (Expression::ScaledSum(x, a, b), c2 @ Expression::Constant(_))
            | (c2 @ Expression::Constant(_), Expression::ScaledSum(x, a, b)) => {
                Expression::ScaledSum(
                    x.clone(),
                    Box::new(a.deref().clone() * c2.clone()),
                    Box::new(b.deref().clone() * c2.clone()),
                )
            }
            // scaled * challenge => scaled
            (Expression::ScaledSum(x, a, b), c2 @ Expression::Challenge(..))
            | (c2 @ Expression::Challenge(..), Expression::ScaledSum(x, a, b)) => {
                Expression::ScaledSum(
                    x.clone(),
                    Box::new(a.deref().clone() * c2.clone()),
                    Box::new(b.deref().clone() * c2.clone()),
                )
            }
            _ => Expression::Product(Box::new(self), Box::new(rhs)),
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct WitIn {
    pub id: WitnessId,
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct Fixed(pub usize);

impl WitIn {
    pub fn from_expr<E: ExtensionField, N, NR>(
        name: N,
        circuit_builder: &mut CircuitBuilder<E>,
        input: Expression<E>,
        debug: bool,
    ) -> Result<Self, ZKVMError>
    where
        NR: Into<String> + Clone,
        N: FnOnce() -> NR,
    {
        circuit_builder.namespace(
            || "from_expr",
            |cb| {
                let name = name().into();
                let wit = cb.create_witin(|| name.clone())?;
                if !debug {
                    cb.require_zero(|| name.clone(), wit.expr() - input)?;
                }
                Ok(wit)
            },
        )
    }

    pub fn assign<E: ExtensionField>(
        &self,
        instance: &mut [MaybeUninit<E::BaseField>],
        value: E::BaseField,
    ) {
        instance[self.id as usize] = MaybeUninit::new(value);
    }
}

#[macro_export]
/// this is to avoid non-monomial expression
macro_rules! create_witin_from_expr {
    // Handle the case for a single expression
    ($name:expr, $builder:expr, $debug:expr, $e:expr) => {
        WitIn::from_expr($name, $builder, $e, $debug)
    };
    // Recursively handle multiple expressions and create a flat tuple with error handling
    ($name:expr, $builder:expr, $debug:expr, $e:expr, $($rest:expr),+) => {
        {
            // Return a Result tuple, handling errors
            Ok::<_, ZKVMError>((WitIn::from_expr($name, $builder, $e, $debug)?, $(WitIn::from_expr($name, $builder, $rest)?),*))
        }
    };
}

pub trait ToExpr<E: ExtensionField> {
    type Output;
    fn expr(&self) -> Self::Output;
}

impl<E: ExtensionField> ToExpr<E> for WitIn {
    type Output = Expression<E>;
    fn expr(&self) -> Expression<E> {
        Expression::WitIn(self.id)
    }
}

impl<F: SmallField, E: ExtensionField<BaseField = F>> ToExpr<E> for F {
    type Output = Expression<E>;
    fn expr(&self) -> Expression<E> {
        Expression::Constant(*self)
    }
}

impl<F: SmallField, E: ExtensionField<BaseField = F>> From<usize> for Expression<E> {
    fn from(value: usize) -> Self {
        Expression::Constant(F::from(value as u64))
    }
}

impl<E: ExtensionField> Display for Expression<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut wtns = vec![];
        write!(f, "{}", fmt::expr(self, &mut wtns, false))
    }
}

pub mod fmt {
    use super::*;
    use std::fmt::Write;

    pub fn expr<E: ExtensionField>(
        expression: &Expression<E>,
        wtns: &mut Vec<WitnessId>,
        add_prn_sum: bool,
    ) -> String {
        match expression {
            Expression::WitIn(wit_in) => {
                wtns.push(*wit_in);
                format!("WitIn({})", wit_in)
            }
            Expression::Challenge(id, pow, scaler, offset) => {
                if *pow == 1 && *scaler == 1.into() && *offset == 0.into() {
                    format!("Challenge({})", id)
                } else {
                    let mut s = String::new();
                    if *scaler != 1.into() {
                        write!(s, "{}*", field(scaler)).unwrap();
                    }
                    write!(s, "Challenge({})", id,).unwrap();
                    if *pow > 1 {
                        write!(s, "^{}", pow).unwrap();
                    }
                    if *offset != 0.into() {
                        write!(s, "+{}", field(offset)).unwrap();
                    }
                    s
                }
            }
            Expression::Constant(constant) => base_field::<E>(constant, true).to_string(),
            Expression::Fixed(fixed) => format!("{:?}", fixed),
            Expression::Sum(left, right) => {
                let s = format!("{} + {}", expr(left, wtns, false), expr(right, wtns, false));
                if add_prn_sum { format!("({})", s) } else { s }
            }
            Expression::Product(left, right) => {
                format!("{} * {}", expr(left, wtns, true), expr(right, wtns, true))
            }
            Expression::ScaledSum(x, a, b) => {
                let s = format!(
                    "{} * {} + {}",
                    expr(a, wtns, true),
                    expr(x, wtns, true),
                    expr(b, wtns, false)
                );
                if add_prn_sum { format!("({})", s) } else { s }
            }
        }
    }

    pub fn field<E: ExtensionField>(field: &E) -> String {
        let name = format!("{:?}", field);
        let name = name.split('(').next().unwrap_or("ExtensionField");

        let data = field
            .as_bases()
            .iter()
            .map(|b| base_field::<E>(b, false))
            .collect::<Vec<String>>();
        let only_one_limb = field.as_bases()[1..].iter().all(|&x| x == 0.into());

        if only_one_limb {
            data[0].to_string()
        } else {
            format!("{name}[{}]", data.join(","))
        }
    }

    pub fn base_field<E: ExtensionField>(base_field: &E::BaseField, add_prn: bool) -> String {
        let value = base_field.to_canonical_u64();

        if value > E::BaseField::MODULUS_U64 - u16::MAX as u64 {
            // beautiful format for negative number > -65536
            prn(format!("-{}", E::BaseField::MODULUS_U64 - value), add_prn)
        } else if value < u16::MAX as u64 {
            format!("{value}")
        } else {
            // hex
            if value > E::BaseField::MODULUS_U64 - (u32::MAX as u64 + u16::MAX as u64) {
                prn(
                    format!("-{:#x}", E::BaseField::MODULUS_U64 - value),
                    add_prn,
                )
            } else {
                format!("{value:#x}")
            }
        }
    }

    pub fn prn(s: String, add_prn: bool) -> String {
        if add_prn { format!("({})", s) } else { s }
    }

    #[cfg(test)]
    pub fn wtns<E: ExtensionField>(
        wtns: &[WitnessId],
        wits_in: &[ArcMultilinearExtension<E>],
        inst_id: usize,
        wits_in_name: &[String],
    ) -> String {
        use itertools::Itertools;

        wtns.iter()
            .sorted()
            .map(|wt_id| {
                let wit = &wits_in[*wt_id as usize];
                let name = &wits_in_name[*wt_id as usize];
                let value_fmt = if let Some(e) = wit.get_ext_field_vec_optn() {
                    field(&e[inst_id])
                } else if let Some(bf) = wit.get_base_field_vec_optn() {
                    base_field::<E>(&bf[inst_id], true)
                } else {
                    "Unknown".to_string()
                };
                format!("  WitIn({wt_id})={value_fmt} {name:?}")
            })
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use goldilocks::GoldilocksExt2;

    use crate::circuit_builder::{CircuitBuilder, ConstraintSystem};

    use super::{fmt, Expression, ToExpr};
    use ff::Field;

    #[test]
    fn test_expression_arithmetics() {
        type E = GoldilocksExt2;
        let mut cs = ConstraintSystem::new(|| "test_root");
        let mut cb = CircuitBuilder::<E>::new(&mut cs);
        let x = cb.create_witin(|| "x").unwrap();

        // scaledsum * challenge
        // 3 * x + 2
        let expr: Expression<E> =
            Into::<Expression<E>>::into(3usize) * x.expr() + Into::<Expression<E>>::into(2usize);
        // c^3 + 1
        let c = Expression::Challenge(0, 3, 1.into(), 1.into());
        // res
        // x* (c^3*3 + 3) + 2c^3 + 2
        assert_eq!(
            c * expr,
            Expression::ScaledSum(
                Box::new(x.expr()),
                Box::new(Expression::Challenge(0, 3, 3.into(), 3.into())),
                Box::new(Expression::Challenge(0, 3, 2.into(), 2.into()))
            )
        );

        // constant * witin
        // 3 * x
        let expr: Expression<E> = Into::<Expression<E>>::into(3usize) * x.expr();
        assert_eq!(
            expr,
            Expression::ScaledSum(
                Box::new(x.expr()),
                Box::new(Expression::Constant(3.into())),
                Box::new(Expression::Constant(0.into()))
            )
        );

        // constant * challenge
        // 3 * (c^3 + 1)
        let expr: Expression<E> = Expression::Constant(3.into());
        let c = Expression::Challenge(0, 3, 1.into(), 1.into());
        assert_eq!(expr * c, Expression::Challenge(0, 3, 3.into(), 3.into()));

        // challenge * challenge
        // (2c^3 + 1) * (2c^2 + 1) = 4c^5 + 2c^3 + 2c^2 + 1
        let res: Expression<E> = Expression::Challenge(0, 3, 2.into(), 1.into())
            * Expression::Challenge(0, 2, 2.into(), 1.into());
        assert_eq!(
            res,
            Expression::Sum(
                Box::new(Expression::Sum(
                    // (s1 * s2 * c1^(pow1 + pow2) + offset1 * offset2
                    Box::new(Expression::Challenge(
                        0,
                        3 + 2,
                        (2 * 2).into(),
                        E::ONE * E::ONE,
                    )),
                    // offset2 * s1 * c1^(pow1)
                    Box::new(Expression::Challenge(0, 3, 2.into(), E::ZERO)),
                )),
                // offset1 * s2 * c2^(pow2))
                Box::new(Expression::Challenge(0, 2, 2.into(), E::ZERO)),
            )
        );
    }

    #[test]
    fn test_is_monomial_form() {
        type E = GoldilocksExt2;
        let mut cs = ConstraintSystem::new(|| "test_root");
        let mut cb = CircuitBuilder::<E>::new(&mut cs);
        let x = cb.create_witin(|| "x").unwrap();
        let y = cb.create_witin(|| "y").unwrap();
        let z = cb.create_witin(|| "z").unwrap();
        // scaledsum * challenge
        // 3 * x + 2
        let expr: Expression<E> =
            Into::<Expression<E>>::into(3usize) * x.expr() + Into::<Expression<E>>::into(2usize);
        assert!(expr.is_monomial_form());

        // 2 product term
        let expr: Expression<E> = Into::<Expression<E>>::into(3usize) * x.expr() * y.expr()
            + Into::<Expression<E>>::into(2usize) * x.expr();
        assert!(expr.is_monomial_form());

        // complex linear operation
        // (2c + 3) * x * y - 6z
        let expr: Expression<E> =
            Expression::Challenge(0, 1, 2.into(), 3.into()) * x.expr() * y.expr()
                - Into::<Expression<E>>::into(6usize) * z.expr();
        assert!(expr.is_monomial_form());

        // complex linear operation
        // (2c + 3) * x * y - 6z
        let expr: Expression<E> =
            Expression::Challenge(0, 1, 2.into(), 3.into()) * x.expr() * y.expr()
                - Into::<Expression<E>>::into(6usize) * z.expr();
        assert!(expr.is_monomial_form());

        // complex linear operation
        // (2 * x + 3) * 3 + 6 * 8
        let expr: Expression<E> = (Into::<Expression<E>>::into(2usize) * x.expr()
            + Into::<Expression<E>>::into(3usize))
            * Into::<Expression<E>>::into(3usize)
            + Into::<Expression<E>>::into(6usize) * Into::<Expression<E>>::into(8usize);
        assert!(expr.is_monomial_form());
    }

    #[test]
    fn test_not_monomial_form() {
        type E = GoldilocksExt2;
        let mut cs = ConstraintSystem::new(|| "test_root");
        let mut cb = CircuitBuilder::<E>::new(&mut cs);
        let x = cb.create_witin(|| "x").unwrap();
        let y = cb.create_witin(|| "y").unwrap();
        // scaledsum * challenge
        // (x + 1) * (y + 1)
        let expr: Expression<E> = (Into::<Expression<E>>::into(1usize) + x.expr())
            * (Into::<Expression<E>>::into(2usize) + y.expr());
        assert!(!expr.is_monomial_form());
    }

    #[test]
    fn test_fmt_expr_challenge_1() {
        let a = Expression::<GoldilocksExt2>::Challenge(0, 2, 3.into(), 4.into());
        let b = Expression::<GoldilocksExt2>::Challenge(0, 5, 6.into(), 7.into());

        let mut wtns_acc = vec![];
        let s = fmt::expr(&(a * b), &mut wtns_acc, false);

        assert_eq!(
            s,
            "18*Challenge(0)^7+28 + 21*Challenge(0)^2 + 24*Challenge(0)^5"
        );
    }

    #[test]
    fn test_fmt_expr_challenge_2() {
        let a = Expression::<GoldilocksExt2>::Challenge(0, 1, 1.into(), 0.into());
        let b = Expression::<GoldilocksExt2>::Challenge(0, 1, 1.into(), 0.into());

        let mut wtns_acc = vec![];
        let s = fmt::expr(&(a * b), &mut wtns_acc, false);

        assert_eq!(s, "Challenge(0)^2");
    }

    #[test]
    fn test_fmt_expr_wtns_acc_1() {
        let expr = Expression::<GoldilocksExt2>::WitIn(0);
        let mut wtns_acc = vec![];
        let s = fmt::expr(&expr, &mut wtns_acc, false);
        assert_eq!(s, "WitIn(0)");
        assert_eq!(wtns_acc, vec![0]);
    }
}
