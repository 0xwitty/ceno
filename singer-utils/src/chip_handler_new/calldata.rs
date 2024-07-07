use std::cell::RefCell;
use std::rc::Rc;
use crate::chip_handler_new::rom_handler::ROMHandler;
use crate::chip_handler_new::util::cell_to_mixed;
use crate::structs::ROMType;
use ff_ext::ExtensionField;
use itertools::Itertools;
use simple_frontend::structs::{CellId, CircuitBuilder, MixedCell};

struct CalldataChip<Ext: ExtensionField> {
    rom_handler: Rc<RefCell<ROMHandler<Ext>>>
}

impl<Ext: ExtensionField> CalldataChip<Ext> {
    // TODO: rename and document
    fn load(
        &self,
        circuit_builder: &mut CircuitBuilder<Ext>,
        offset: &[CellId],
        data: &[CellId],
    ) {
        let key = [
            vec![MixedCell::Constant(Ext::BaseField::from(
                ROMType::Calldata as u64,
            ))],
            cell_to_mixed(offset),
        ]
        .concat();
        let data = data.iter().map(|&x| x.into()).collect_vec();
        self.rom_handler.borrow_mut().read_mixed(circuit_builder, &key, &data);
    }
}
