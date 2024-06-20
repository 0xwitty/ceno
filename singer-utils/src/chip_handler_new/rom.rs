use crate::structs::ChipChallenges;
use ff::Field;
use ff_ext::ExtensionField;
use simple_frontend::structs::{CellId, CircuitBuilder, ExtCellId, MixedCell, WitnessId};

// TODO: add documentation
pub struct ROM<Ext: ExtensionField> {
    records: Vec<ExtCellId<Ext>>,
    challenge: ChipChallenges,
}

impl<Ext: ExtensionField> ROM<Ext> {
    // TODO: add documentation
    // TODO: can this be named read?
    fn load(
        &mut self,
        circuit_builder: &mut CircuitBuilder<Ext>,
        key: &[CellId],
        value: &[CellId],
    ) {
        let item_rlc = circuit_builder.create_ext_cell();
        let items = [key.to_vec(), value.to_vec()].concat();
        circuit_builder.rlc(&item_rlc, &items, self.challenge.record_item_rlc());

        let out = circuit_builder.create_ext_cell();
        circuit_builder.rlc_ext(&out, &[item_rlc], self.challenge.record_rlc());
        self.records.push(out);
    }

    // TODO: add documentation
    fn load_mixed(
        &mut self,
        circuit_builder: &mut CircuitBuilder<Ext>,
        key: &[MixedCell<Ext>],
        value: &[MixedCell<Ext>],
    ) {
        let item_rlc = circuit_builder.create_ext_cell();
        let items = [key.to_vec(), value.to_vec()].concat();
        circuit_builder.rlc_mixed(&item_rlc, &items, self.challenge.record_item_rlc());

        let out = circuit_builder.create_ext_cell();
        circuit_builder.rlc_ext(&out, &[item_rlc], self.challenge.record_rlc());
        self.records.push(out);
    }

    // TODO: add documentation
    // what is this supposed to return??
    // it seems to pad it to the next highest power of two (with empty cells)
    // then generates a witness Id for them (type out)
    // frac function is here: construct_chip_check_graph
    fn finalize(self, circuit_builder: &mut CircuitBuilder<Ext>) -> Option<(WitnessId, usize)> {
        if self.records.len() == 0 {
            return None;
        }

        let padding_count = self.records.len().next_power_of_two() - self.records.len();
        let last_cell = self.records.last().expect("confirmed records.len() > 0");
        let mut records = self.records.clone();

        for _ in 0..padding_count {
            let out = circuit_builder.create_ext_cell();
            circuit_builder.add_ext(&out, last_cell, Ext::BaseField::ONE);
            records.push(out);
        }

        Some((
            circuit_builder.create_witness_out_from_exts(&records),
            records.len(),
        ))
    }
}
