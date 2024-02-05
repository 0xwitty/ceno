use frontend::structs::{CellId, CircuitBuilder, ConstantType};
use goldilocks::SmallField;

use crate::{
    error::ZKVMError,
    instructions::utils::{ChipHandler, UInt},
};

use super::UIntAddSub;

impl<const M: usize, const C: usize> UIntAddSub<UInt<M, C>> {
    pub(in crate::instructions) const N_NO_OVERFLOW_WITNESS_CELLS: usize =
        UInt::<M, C>::N_RANGE_CHECK_CELLS + UInt::<M, C>::N_CARRY_NO_OVERFLOW_CELLS;
    pub(in crate::instructions) const N_NO_OVERFLOW_WITNESS_UNSAFE_CELLS: usize =
        UInt::<M, C>::N_CARRY_NO_OVERFLOW_CELLS;

    pub(in crate::instructions) const N_WITNESS_UNSAFE_CELLS: usize = UInt::<M, C>::N_CARRY_CELLS;
    pub(in crate::instructions) const N_WITNESS_CELLS: usize =
        UInt::<M, C>::N_RANGE_CHECK_CELLS + UInt::<M, C>::N_CARRY_CELLS;

    pub(in crate::instructions) fn extract_range_values(witness: &[CellId]) -> &[CellId] {
        &witness[..UInt::<M, C>::N_RANGE_CHECK_CELLS]
    }

    pub(in crate::instructions) fn extract_range_values_no_overflow(
        witness: &[CellId],
    ) -> &[CellId] {
        &witness[..UInt::<M, C>::N_RANGE_CHECK_NO_OVERFLOW_CELLS]
    }

    pub(in crate::instructions) fn extract_carry_no_overflow(witness: &[CellId]) -> &[CellId] {
        &witness[UInt::<M, C>::N_RANGE_CHECK_NO_OVERFLOW_CELLS..]
    }

    pub(in crate::instructions) fn extract_carry(witness: &[CellId]) -> &[CellId] {
        &witness[UInt::<M, C>::N_RANGE_CHECK_CELLS..]
    }

    pub(in crate::instructions) fn extract_unsafe_carry(witness: &[CellId]) -> &[CellId] {
        witness
    }

    /// Little-endian addition. Assume users to check the correct range of the
    /// result by themselves.
    pub(in crate::instructions) fn add_unsafe<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        addend_0: &UInt<M, C>,
        addend_1: &UInt<M, C>,
        carry: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let result: UInt<M, C> = circuit_builder
            .create_cells(UInt::<M, C>::N_OPRAND_CELLS)
            .try_into()?;
        for i in 0..UInt::<M, C>::N_OPRAND_CELLS {
            let (a, b, result) = (addend_0.values[i], addend_1.values[i], result.values[i]);
            // result = addend_0 + addend_1 + last_carry - carry * (1 << VALUE_BIT_WIDTH)
            circuit_builder.add(result, a, ConstantType::Field(F::ONE));
            circuit_builder.add(result, b, ConstantType::Field(F::ONE));
            // It is equivalent to pad carry with 0s.
            if i < carry.len() {
                circuit_builder.add(result, carry[i], ConstantType::Field(-F::from(1 << C)));
            }
            if i > 0 && i - 1 < carry.len() {
                circuit_builder.add(result, carry[i - 1], ConstantType::Field(F::ONE));
            }
        }
        Ok(result)
    }

    /// Little-endian addition.
    pub(in crate::instructions) fn add<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        range_chip_handler: &mut ChipHandler,
        addend_0: &UInt<M, C>,
        addend_1: &UInt<M, C>,
        witness: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let carry = Self::extract_carry(witness);
        let range_values = Self::extract_range_values(witness);
        let computed_result = Self::add_unsafe(circuit_builder, addend_0, addend_1, carry)?;
        range_chip_handler.range_check_uint(circuit_builder, &computed_result, Some(range_values))
    }

    /// Little-endian addition with a constant. Assume users to check the
    /// correct range of the result by themselves.
    pub(in crate::instructions) fn add_const_unsafe<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        addend_0: &UInt<M, C>,
        constant: &F,
        carry: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let result: UInt<M, C> = circuit_builder
            .create_cells(UInt::<M, C>::N_OPRAND_CELLS)
            .try_into()?;
        for i in 0..result.values.len() {
            let (a, result) = (addend_0.values[i], result.values[i]);
            // result = addend_0 + addend_1 + last_carry - carry * (256 << BYTE_WIDTH)
            circuit_builder.add(result, a, ConstantType::Field(F::ONE));
            circuit_builder.add_const(result, ConstantType::Field(*constant));
            // It is equivalent to pad carry with 0s.
            if i < carry.len() {
                circuit_builder.add(result, carry[i], ConstantType::Field(-F::from(1 << C)));
            }
            if i > 0 && i - 1 < carry.len() {
                circuit_builder.add(result, carry[i - 1], ConstantType::Field(F::ONE));
            }
        }
        Ok(result)
    }

    /// Little-endian addition with a constant.
    pub(in crate::instructions) fn add_const<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        range_chip_handler: &mut ChipHandler,
        addend_0: &UInt<M, C>,
        constant: &F,
        witness: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let carry = Self::extract_carry(witness);
        let range_values = Self::extract_range_values(witness);
        let computed_result = Self::add_const_unsafe(circuit_builder, addend_0, constant, carry)?;
        range_chip_handler.range_check_uint(circuit_builder, &computed_result, Some(range_values))
    }

    /// Little-endian addition with a constant, guaranteed no overflow.
    pub(in crate::instructions) fn add_const_no_overflow<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        range_chip_handler: &mut ChipHandler,
        addend_0: &UInt<M, C>,
        constant: &F,
        witness: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let carry = Self::extract_carry_no_overflow(witness);
        let range_values = Self::extract_range_values_no_overflow(witness);
        let computed_result = Self::add_const_unsafe(circuit_builder, addend_0, constant, carry)?;
        range_chip_handler.range_check_uint(circuit_builder, &computed_result, Some(range_values))
    }

    /// Little-endian addition with a small number. Notice that the user should
    /// guarantee addend_1 < 1 << C.
    pub(in crate::instructions) fn add_small_unsafe<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        addend_0: &UInt<M, C>,
        addend_1: CellId,
        carry: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let result: UInt<M, C> = circuit_builder
            .create_cells(UInt::<M, C>::N_OPRAND_CELLS)
            .try_into()?;
        for i in 0..result.values.len() {
            let (a, result) = (addend_0.values[i], result.values[i]);
            // result = addend_0 + addend_1 + last_carry - carry * (256 << BYTE_WIDTH)
            circuit_builder.add(result, a, ConstantType::Field(F::ONE));
            circuit_builder.add(result, addend_1, ConstantType::Field(F::ONE));
            // It is equivalent to pad carry with 0s.
            if i < carry.len() {
                circuit_builder.add(result, carry[i], ConstantType::Field(-F::from(1 << C)));
            }
            if i > 0 && i - 1 < carry.len() {
                circuit_builder.add(result, carry[i - 1], ConstantType::Field(F::ONE));
            }
        }
        Ok(result)
    }

    /// Little-endian addition with a small number. Notice that the user should
    /// guarantee addend_1 < 1 << C.
    pub(in crate::instructions) fn add_small<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        range_chip_handler: &mut ChipHandler,
        addend_0: &UInt<M, C>,
        addend_1: CellId,
        witness: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let carry = Self::extract_carry(witness);
        let range_values = Self::extract_range_values(witness);
        let computed_result = Self::add_small_unsafe(circuit_builder, addend_0, addend_1, carry)?;
        range_chip_handler.range_check_uint(circuit_builder, &computed_result, Some(range_values))
    }

    /// Little-endian addition with a small number, guaranteed no overflow.
    /// Notice that the user should guarantee addend_1 < 1 << C.
    pub(in crate::instructions) fn add_small_no_overflow<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        range_chip_handler: &mut ChipHandler,
        addend_0: &UInt<M, C>,
        addend_1: CellId,
        witness: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let carry = Self::extract_carry_no_overflow(witness);
        let range_values = Self::extract_range_values_no_overflow(witness);
        let computed_result = Self::add_small_unsafe(circuit_builder, addend_0, addend_1, carry)?;
        range_chip_handler.range_check_uint(circuit_builder, &computed_result, Some(range_values))
    }

    /// Little-endian subtraction. Assume users to check the correct range of
    /// the result by themselves.
    pub(in crate::instructions) fn sub_unsafe<F: SmallField>(
        circuit_builder: &mut CircuitBuilder<F>,
        minuend: &UInt<M, C>,
        subtrahend: &UInt<M, C>,
        borrow: &[CellId],
    ) -> Result<UInt<M, C>, ZKVMError> {
        let result: UInt<M, C> = circuit_builder
            .create_cells(UInt::<M, C>::N_OPRAND_CELLS)
            .try_into()?;
        // result = minuend - subtrahend + borrow * (1 << BIT_WIDTH) - last_borrow
        for i in 0..result.values.len() {
            let (minuend, subtrahend, result) =
                (minuend.values[i], subtrahend.values[i], result.values[i]);
            circuit_builder.add(result, minuend, ConstantType::Field(F::ONE));
            circuit_builder.add(result, subtrahend, ConstantType::Field(-F::ONE));
            if i < borrow.len() {
                circuit_builder.add(result, borrow[i], ConstantType::Field(F::from(1 << C)));
            }
            if i > 0 && i - 1 < borrow.len() {
                circuit_builder.add(result, borrow[i - 1], ConstantType::Field(-F::ONE));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use super::{UInt, UIntAddSub};
    use frontend::structs::CircuitBuilder;
    use gkr::structs::{Circuit, CircuitWitness};
    use goldilocks::Goldilocks;

    #[test]
    fn test_add_unsafe() {
        type Uint256_8 = UInt<256, 8>;
        assert_eq!(Uint256_8::N_OPRAND_CELLS, 32);
        let mut circuit_builder = CircuitBuilder::<Goldilocks>::new();

        // configure circuit with cells for addend_0, addend_1 and carry as wire_in
        let (addend_0_wire_in_id, addend_0_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let (addend_1_wire_in_id, addend_1_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let (carry_wire_in_id, carry_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let addend_0 = Uint256_8::try_from(addend_0_wire_in_cells);
        let addend_1 = Uint256_8::try_from(addend_1_wire_in_cells);
        let result = UIntAddSub::<Uint256_8>::add_unsafe(
            &mut circuit_builder,
            &addend_0.unwrap(),
            &addend_1.unwrap(),
            &carry_wire_in_cells,
        );
        assert_eq!(result.unwrap().values(), (96..128).collect::<Vec<usize>>());
        circuit_builder.configure();
        let circuit = Circuit::new(&circuit_builder);
        //println!("add unsafe circuit {:?}", circuit);

        // generate witnesses for addend_0, addend_1 and carry
        // must pad each witness to the size of N_OPERAND_CELLS
        let n_wires_in = circuit.n_wires_in;
        let mut wires_in = vec![vec![]; n_wires_in];
        wires_in[addend_0_wire_in_id as usize] =
            vec![Goldilocks::from(255u64), Goldilocks::from(255u64)];
        wires_in[addend_0_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        wires_in[addend_1_wire_in_id as usize] =
            vec![Goldilocks::from(255u64), Goldilocks::from(254u64)];
        wires_in[addend_1_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        wires_in[carry_wire_in_id as usize] = vec![Goldilocks::from(1u64), Goldilocks::from(1u64)];
        wires_in[carry_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        let circuit_witness = {
            let challenges = vec![Goldilocks::from(2)];
            let mut circuit_witness = CircuitWitness::new(&circuit, challenges);
            circuit_witness.add_instance(&circuit, &wires_in);
            circuit_witness
        };
        //println!("{:?}", circuit_witness);
        circuit_witness.check_correctness(&circuit);

        // check the result
        let result_values = circuit_witness.last_layer_witness_ref();
        //println!("{:?}", result_values[0]);
        assert_eq!(result_values[0][0], Goldilocks::from(254u64));
        assert_eq!(result_values[0][1], Goldilocks::from(254u64));
        assert_eq!(result_values[0][2], Goldilocks::from(1u64));
    }

    #[test]
    fn test_sub_unsafe() {
        type Uint256_8 = UInt<256, 8>;
        assert_eq!(Uint256_8::N_OPRAND_CELLS, 32);
        let mut circuit_builder = CircuitBuilder::<Goldilocks>::new();

        // configure circuit with cells for minuend, subtrend and borrow as wire_in
        let (minuend_wire_in_id, minuend_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let (subtrend_wire_in_id, subtrend_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let (borrow_wire_in_id, borrow_wire_in_cells) =
            circuit_builder.create_wire_in(Uint256_8::N_OPRAND_CELLS);
        let minuend = Uint256_8::try_from(minuend_wire_in_cells);
        let subtrend = Uint256_8::try_from(subtrend_wire_in_cells);
        let result = UIntAddSub::<Uint256_8>::sub_unsafe(
            &mut circuit_builder,
            &minuend.unwrap(),
            &subtrend.unwrap(),
            &borrow_wire_in_cells,
        );
        assert_eq!(result.unwrap().values(), (96..128).collect::<Vec<usize>>());
        circuit_builder.configure();
        let circuit = Circuit::new(&circuit_builder);
        //println!("add unsafe circuit {:?}", circuit);

        // generate witnesses for addend_0, addend_1 and carry
        // must pad each witness to the size of N_OPERAND_CELLS
        let n_wires_in = circuit.n_wires_in;
        let mut wires_in = vec![vec![]; n_wires_in];
        wires_in[minuend_wire_in_id as usize] =
            vec![Goldilocks::from(1u64), Goldilocks::from(1u64)];
        wires_in[minuend_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        wires_in[subtrend_wire_in_id as usize] =
            vec![Goldilocks::from(255u64), Goldilocks::from(254u64)];
        wires_in[subtrend_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        wires_in[borrow_wire_in_id as usize] = vec![Goldilocks::from(1u64), Goldilocks::from(1u64)];
        wires_in[borrow_wire_in_id as usize]
            .extend(vec![Goldilocks::from(0u64); Uint256_8::N_OPRAND_CELLS - 2]);
        let circuit_witness = {
            let challenges = vec![Goldilocks::from(2)];
            let mut circuit_witness = CircuitWitness::new(&circuit, challenges);
            circuit_witness.add_instance(&circuit, &wires_in);
            circuit_witness
        };
        //println!("{:?}", circuit_witness);
        circuit_witness.check_correctness(&circuit);

        // check the result
        let result_values = circuit_witness.last_layer_witness_ref();
        //println!("{:?}", result_values[0]);
        assert_eq!(result_values[0][0], Goldilocks::from(2u64));
        assert_eq!(result_values[0][1], Goldilocks::from(2u64));
        assert_eq!(result_values[0][2], -Goldilocks::from(1u64));
    }
}
