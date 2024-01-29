macro_rules! register_wires_in {
    ($struct_name:ident, $($wire_name:ident { $($slice_name:ident => $length:expr),* }),*) => {
        impl $struct_name {
            $(
                #[inline]
                pub fn $wire_name() -> usize {
                    (0 $(+ $length)* as usize).next_power_of_two()
                }

                register_wires_in!(@internal $wire_name, 0usize; $($slice_name => $length),*);
            )*
        }
    };

    ($struct_name:ident<N>, $($wire_name:ident { $($slice_name:ident => $length:expr),* }),*) => {
        impl<const N: usize> $struct_name<N> {
            $(
                #[inline]
                pub fn $wire_name() -> usize {
                    (0 $(+ $length)* as usize).next_power_of_two()
                }

                register_wires_in!(@internal $wire_name, 0usize; $($slice_name => $length),*);
            )*
        }
    };

    (@internal $wire_name:ident, $offset:expr; $name:ident => $length:expr $(, $rest:ident => $rest_length:expr)*) => {
        fn $name() -> std::ops::Range<usize> {
            $offset..$offset + $length
        }
        register_wires_in!(@internal $wire_name, $offset + $length; $($rest => $rest_length),*);
    };

    (@internal $wire_name:ident, $offset:expr;) => {};
}

macro_rules! register_wires_out {
    ($struct_name:ident, $($wire_name:ident { $($slice_name:ident => $length:expr),* }),*) => {
        impl $struct_name {
            $(
                #[inline]
                pub fn $wire_name() -> usize {
                    (0 $(+ $length)* as usize).next_power_of_two()
                }
            )*
        }
    };

    ($struct_name:ident<N>, $($wire_name:ident { $($slice_name:ident => $length:expr),* }),*) => {
        impl<const N: usize> $struct_name<N> {
            $(
                #[inline]
                pub fn $wire_name() -> usize {
                    (0 $(+ $length)* as usize).next_power_of_two()
                }
            )*
        }
    };
}

macro_rules! register_succ_wire_out {
    ($struct_name:ident, $($succ_name:ident),*) => {
        impl $struct_name {
            register_succ_wire_out!(@internal 0usize; $($succ_name),*);
        }
    };

    (@internal $offset:expr; $name:ident $(, $rest:ident)*) => {
        fn $name() -> usize {
            $offset
        }
        register_succ_wire_out!(@internal $offset + 1; $($rest),*);
    };

    (@internal $offset:expr;) => {
        fn succ_wire_out_num() -> usize {
            $offset
        }
    };
}