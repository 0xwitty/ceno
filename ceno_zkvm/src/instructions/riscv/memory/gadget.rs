use crate::{
    Value,
    circuit_builder::CircuitBuilder,
    error::ZKVMError,
    expression::{Expression, ToExpr, WitIn},
    instructions::riscv::{constants::UInt, insn_base::MemAddr},
    set_val,
    witness::LkMultiplicity,
};
use ceno_emul::StepRecord;
use ff_ext::ExtensionField;
use std::mem::MaybeUninit;

pub struct MemWordChange<const N_ZEROS: usize> {
    // decompose limb into bytes iff N_ZEROS == 0
    prev_limb_bytes: Vec<WitIn>,
    rs2_limb_bytes: Vec<WitIn>,

    // its length + N_ZEROS equals to 2
    expected_changes: Vec<WitIn>,
}

impl<const N_ZEROS: usize> MemWordChange<N_ZEROS> {
    pub(crate) fn construct_circuit<E: ExtensionField>(
        cb: &mut CircuitBuilder<E>,
        addr: &MemAddr<E>,
        prev_word: &UInt<E>,
        rs2_word: &UInt<E>,
    ) -> Result<Self, ZKVMError> {
        let select =
            |bit: &Expression<E>, when_true: &Expression<E>, when_false: &Expression<E>| {
                bit.clone() * when_true.clone()
                    + (E::BaseField::from(1).expr() - bit.clone()) * when_false.clone()
            };

        let mut decompose_limb = |limb_anno: &str,
                                  limb: &Expression<E>,
                                  num_bytes: usize|
         -> Result<Vec<WitIn>, ZKVMError> {
            let bytes = (0..num_bytes)
                .map(|i| cb.create_witin(|| format!("{}.le_bytes[{}]", limb_anno, i)))
                .collect::<Result<Vec<WitIn>, ZKVMError>>()?;

            cb.require_equal(
                || format!("decompose {} into {} bytes", limb_anno, num_bytes),
                limb.clone(),
                bytes
                    .iter()
                    .enumerate()
                    .fold(Expression::ZERO, |acc, (idx, byte)| {
                        acc + E::BaseField::from(1 << (idx * 8)).expr() * byte.expr()
                    }),
            )?;

            Ok(bytes)
        };

        // for sb (n_zeros = 0)
        match N_ZEROS {
            0 => {
                assert!(prev_word.wits_in().is_some() && rs2_word.wits_in().is_some());

                let low_bits = addr.low_bit_exprs();
                let prev_limbs = prev_word.expr();
                let rs2_limbs = rs2_word.expr();

                // degree == 2
                let prev_target_limb = select(&low_bits[1], &prev_limbs[1], &prev_limbs[0]);
                let rs2_target_limb = select(&low_bits[1], &rs2_limbs[1], &rs2_limbs[0]);

                let prev_limb_bytes = decompose_limb("prev_limb", &prev_target_limb, 2)?;
                let rs2_limb_bytes = decompose_limb("rs2_limb", &rs2_target_limb, 2)?;

                let expected_limb_change = cb.create_witin(|| "expected_limb_change")?;
                cb.require_equal(
                    || "expected_limb_change = select(low_bits[0], rs2 - prev)",
                    // degree 2 expression
                    select(
                        &low_bits[0],
                        &(E::BaseField::from(1 << 8).expr()
                            * (rs2_limb_bytes[1].expr() - prev_limb_bytes[1].expr())),
                        &(E::BaseField::from(1).expr()
                            * (rs2_limb_bytes[0].expr() - prev_limb_bytes[0].expr())),
                    ),
                    expected_limb_change.expr(),
                )?;

                let expected_change = cb.create_witin(|| "expected_change")?;
                cb.require_equal(
                    || "expected_change = select(low_bits[1], limb_change*2^16, limb_change)",
                    // degree 2 expression
                    select(
                        &low_bits[1],
                        &(E::BaseField::from(1 << 16).expr() * expected_limb_change.expr()),
                        &(E::BaseField::from(1).expr() * expected_limb_change.expr()),
                    ),
                    expected_change.expr(),
                )?;

                Ok(MemWordChange {
                    prev_limb_bytes,
                    rs2_limb_bytes,
                    expected_changes: vec![expected_limb_change, expected_change],
                })
            }
            // for sh (n_zeros = 1)
            1 => {
                assert!(prev_word.wits_in().is_some() && rs2_word.wits_in().is_some());

                let low_bits = addr.low_bit_exprs();
                let prev_limbs = prev_word.expr();
                let rs2_limbs = rs2_word.expr();

                let expected_change = cb.create_witin(|| "expected_change")?;

                cb.require_equal(
                    || "expected_change = select(low_bits[1], 2^16*(limb_change))",
                    // degree 2 expression
                    select(
                        &low_bits[1],
                        &(E::BaseField::from(1 << 16).expr()
                            * (rs2_limbs[1].clone() - prev_limbs[1].clone())),
                        &(E::BaseField::from(1).expr()
                            * (rs2_limbs[1].clone() - prev_limbs[0].clone())),
                    ),
                    expected_change.expr(),
                )?;

                Ok(MemWordChange {
                    prev_limb_bytes: vec![],
                    rs2_limb_bytes: vec![],
                    expected_changes: vec![expected_change],
                })
            }
            _ => unreachable!("N_ZEROS cannot be larger than 1"),
        }
    }

    pub(crate) fn value<E: ExtensionField>(&self) -> Expression<E> {
        assert!(N_ZEROS <= 1);

        self.expected_changes[1 - N_ZEROS].expr()
    }

    pub fn assign_instance<E: ExtensionField>(
        &self,
        instance: &mut [MaybeUninit<E::BaseField>],
        lk_multiplicity: &mut LkMultiplicity,
        step: &StepRecord,
    ) -> Result<(), ZKVMError> {
        // memory_addr, prev_value, rs2_value,
        let memory_op = step.memory_op().clone().unwrap();
        let prev_value = Value::new(memory_op.value.before, lk_multiplicity);
        let rs2_value = Value::new_unchecked(step.rs2().unwrap().value);

        assert!(memory_op.shift <= 0x03);

        let low_bits = [memory_op.shift & 1, (memory_op.shift >> 1) & 1];
        let prev_limb = prev_value.as_u16_limbs()[low_bits[1] as usize];
        let rs2_limb = rs2_value.as_u16_limbs()[low_bits[1] as usize];

        self.prev_limb_bytes
            .iter()
            .zip(prev_limb.to_le_bytes())
            .for_each(|(col, byte)| {
                set_val!(instance, *col, E::BaseField::from(byte as u64));
                lk_multiplicity.assert_ux::<8>(byte as u64);
            });

        self.rs2_limb_bytes
            .iter()
            .zip(rs2_limb.to_le_bytes())
            .for_each(|(col, byte)| {
                set_val!(instance, *col, E::BaseField::from(byte as u64));
                lk_multiplicity.assert_ux::<8>(byte as u64);
            });

        match N_ZEROS {
            0 => {
                let change = if low_bits[0] == 0 {
                    E::BaseField::from(rs2_limb.to_le_bytes()[0] as u64)
                        - E::BaseField::from(prev_limb.to_le_bytes()[0] as u64)
                } else {
                    E::BaseField::from((rs2_limb.to_le_bytes()[1] as u64) << 8)
                        - E::BaseField::from((rs2_limb.to_le_bytes()[1] as u64) << 8)
                };
                let final_change = if low_bits[1] == 0 {
                    change
                } else {
                    E::BaseField::from(1u64 << 16) * change
                };
                set_val!(instance, self.expected_changes[0], change);
                set_val!(instance, self.expected_changes[1], final_change);
            }
            1 => {
                let final_change = if low_bits[1] == 0 {
                    E::BaseField::from(rs2_limb as u64) - E::BaseField::from(prev_limb as u64)
                } else {
                    E::BaseField::from((rs2_limb as u64) << 16)
                        - E::BaseField::from((prev_limb as u64) << 16)
                };
                set_val!(instance, self.expected_changes[0], final_change);
            }
            _ => unreachable!("N_ZEROS cannot be larger than 1"),
        }

        Ok(())
    }
}