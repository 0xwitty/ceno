use crate::chip_handler_new::rom_handler::ROMHandler;
use crate::structs::ROMType;
use ff_ext::ExtensionField;
use itertools::Itertools;
use simple_frontend::structs::{CellId, CircuitBuilder, MixedCell};

// TODO: consider chip instantiation with the rom_handler then use later
//  for all chips
struct CalldataChip {}

impl CalldataChip {
    // TODO: rename and document
    fn load<Ext: ExtensionField>(
        &mut self,
        rom_handler: &mut ROMHandler<Ext>,
        circuit_builder: &mut CircuitBuilder<Ext>,
        offset: &[CellId],
        data: &[CellId],
    ) {
        let key = [
            vec![MixedCell::Constant(Ext::BaseField::from(
                ROMType::Calldata as u64,
            ))],
            // TODO: should be able to implement a helper method on &[CellId]
            //  to avoid this duplicated sequence
            offset.iter().map(|&x| x.into()).collect_vec(),
        ]
        .concat();
        let data = data.iter().map(|&x| x.into()).collect_vec();
        rom_handler.read_mixed(circuit_builder, &key, &data);
    }
}
