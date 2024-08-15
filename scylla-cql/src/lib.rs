pub mod errors;
pub mod frame;
#[macro_use]
pub mod macros {
    pub use scylla_macros::DeserializeRow;
    pub use scylla_macros::DeserializeValue;
    pub use scylla_macros::FromRow;
    pub use scylla_macros::FromUserType;
    pub use scylla_macros::IntoUserType;
    pub use scylla_macros::SerializeRow;
    pub use scylla_macros::SerializeValue;
    pub use scylla_macros::ValueList;

    // Reexports for derive(IntoUserType)
    pub use bytes::{BufMut, Bytes, BytesMut};

    pub use crate::impl_from_cql_value_from_method;
}

pub mod types;

pub use crate::frame::response::cql_to_rust;
pub use crate::frame::response::cql_to_rust::FromRow;

pub use crate::frame::types::Consistency;

#[doc(hidden)]
pub mod _macro_internal {
    pub use crate::frame::response::cql_to_rust::{
        FromCqlVal, FromCqlValError, FromRow, FromRowError,
    };
    pub use crate::frame::response::result::{ColumnSpec, ColumnType, CqlValue, DropOptimizedVec, Row};
    pub use crate::frame::value::{
        LegacySerializedValues, SerializedResult, Value, ValueList, ValueTooBig,
    };
    pub use crate::macros::*;

    pub use crate::types::deserialize::row::{
        deser_error_replace_rust_name as row_deser_error_replace_rust_name,
        mk_deser_err as mk_row_deser_err, mk_typck_err as mk_row_typck_err,
        BuiltinDeserializationError as BuiltinRowDeserializationError,
        BuiltinDeserializationErrorKind as BuiltinRowDeserializationErrorKind,
        BuiltinTypeCheckErrorKind as DeserBuiltinRowTypeCheckErrorKind, ColumnIterator,
        DeserializeRow,
    };
    pub use crate::types::deserialize::value::{
        deser_error_replace_rust_name as value_deser_error_replace_rust_name,
        mk_deser_err as mk_value_deser_err, mk_typck_err as mk_value_typck_err,
        BuiltinDeserializationError as BuiltinTypeDeserializationError,
        BuiltinDeserializationErrorKind as BuiltinTypeDeserializationErrorKind,
        BuiltinTypeCheckErrorKind as DeserBuiltinTypeTypeCheckErrorKind, DeserializeValue,
        UdtDeserializationErrorKind, UdtIterator,
        UdtTypeCheckErrorKind as DeserUdtTypeCheckErrorKind,
    };
    pub use crate::types::deserialize::{DeserializationError, FrameSlice, TypeCheckError};
    pub use crate::types::serialize::row::{
        BuiltinSerializationError as BuiltinRowSerializationError,
        BuiltinSerializationErrorKind as BuiltinRowSerializationErrorKind,
        BuiltinTypeCheckError as BuiltinRowTypeCheckError,
        BuiltinTypeCheckErrorKind as BuiltinRowTypeCheckErrorKind, RowSerializationContext,
        SerializeRow,
    };
    pub use crate::types::serialize::value::{
        BuiltinSerializationError as BuiltinTypeSerializationError,
        BuiltinSerializationErrorKind as BuiltinTypeSerializationErrorKind,
        BuiltinTypeCheckError as BuiltinTypeTypeCheckError,
        BuiltinTypeCheckErrorKind as BuiltinTypeTypeCheckErrorKind, SerializeValue,
        UdtSerializationErrorKind, UdtTypeCheckErrorKind,
    };
    pub use crate::types::serialize::writers::WrittenCellProof;
    pub use crate::types::serialize::{
        CellValueBuilder, CellWriter, RowWriter, SerializationError,
    };
}
