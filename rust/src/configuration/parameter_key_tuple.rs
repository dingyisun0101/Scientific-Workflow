//! Sealed tuple decoding for resolved task parameters.

use serde::de::DeserializeOwned;

use super::{ConfigurationError, TaskParameters};

mod sealed {
    pub trait Sealed<Values> {}
}

/// Internal mapping used by [`TaskParameters::decode_values`].
///
/// This trait is public only because it appears in a public method bound. It is
/// sealed, omitted from the prelude, and implemented for borrowed key tuples
/// with arities two through twelve.
#[doc(hidden)]
pub trait ParameterKeyTuple<Values>: sealed::Sealed<Values> {
    /// Decodes the supported tuple from one resolved parameter dictionary.
    #[doc(hidden)]
    fn decode(self, parameters: &TaskParameters) -> Result<Values, ConfigurationError>;
}

macro_rules! key_type {
    ($_type:ident, $lifetime:lifetime) => {
        &$lifetime str
    };
}

macro_rules! impl_parameter_key_tuple {
    ($(($type:ident, $key:ident)),+ $(,)?) => {
        impl<'key, $($type),+> sealed::Sealed<($($type,)+)>
            for ($(key_type!($type, 'key),)+)
        where
            $($type: DeserializeOwned,)+
        {
        }

        impl<'key, $($type),+> ParameterKeyTuple<($($type,)+)>
            for ($(key_type!($type, 'key),)+)
        where
            $($type: DeserializeOwned,)+
        {
            fn decode(
                self,
                parameters: &TaskParameters,
            ) -> Result<($($type,)+), ConfigurationError> {
                let ($($key,)+) = self;
                Ok(($(parameters.decode_value::<$type>($key)?,)+))
            }
        }
    };
}

impl_parameter_key_tuple!((A, key_a), (B, key_b));
impl_parameter_key_tuple!((A, key_a), (B, key_b), (C, key_c));
impl_parameter_key_tuple!((A, key_a), (B, key_b), (C, key_c), (D, key_d));
impl_parameter_key_tuple!((A, key_a), (B, key_b), (C, key_c), (D, key_d), (E, key_e));
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
    (H, key_h),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
    (H, key_h),
    (I, key_i),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
    (H, key_h),
    (I, key_i),
    (J, key_j),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
    (H, key_h),
    (I, key_i),
    (J, key_j),
    (K, key_k),
);
impl_parameter_key_tuple!(
    (A, key_a),
    (B, key_b),
    (C, key_c),
    (D, key_d),
    (E, key_e),
    (F, key_f),
    (G, key_g),
    (H, key_h),
    (I, key_i),
    (J, key_j),
    (K, key_k),
    (L, key_l),
);
