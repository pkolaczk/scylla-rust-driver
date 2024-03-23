//! Provides types for dealing with CQL value deserialization.

use bytes::Bytes;

use std::fmt::Display;

use thiserror::Error;

use super::{DeserializationError, FrameSlice, TypeCheckError};
use crate::frame::frame_errors::ParseError;
use crate::frame::response::result::{deser_cql_value, ColumnType, CqlValue};
use crate::frame::types;
use crate::frame::value::{
    Counter, CqlDate, CqlDecimal, CqlDuration, CqlTime, CqlTimestamp, CqlVarint,
};

/// A type that can be deserialized from a column value inside a row that was
/// returned from a query.
///
/// For tips on how to write a custom implementation of this trait, see the
/// documentation of the parent module.
///
/// The crate also provides a derive macro which allows to automatically
/// implement the trait for a custom type. For more details on what the macro
/// is capable of, see its documentation.
pub trait DeserializeValue<'frame>
where
    Self: Sized,
{
    /// Checks that the column type matches what this type expects.
    fn type_check(typ: &ColumnType) -> Result<(), TypeCheckError>;

    /// Deserialize a column value from given serialized representation.
    ///
    /// This function can assume that the driver called `type_check` to verify
    /// the column's type. Note that `deserialize` is not an unsafe function,
    /// so it should not use the assumption about `type_check` being called
    /// as an excuse to run `unsafe` code.
    fn deserialize(
        typ: &'frame ColumnType,
        v: Option<FrameSlice<'frame>>,
    ) -> Result<Self, DeserializationError>;
}

impl<'frame> DeserializeValue<'frame> for CqlValue {
    fn type_check(_typ: &ColumnType) -> Result<(), TypeCheckError> {
        // CqlValue accepts all possible CQL types
        Ok(())
    }

    fn deserialize(
        typ: &'frame ColumnType,
        v: Option<FrameSlice<'frame>>,
    ) -> Result<Self, DeserializationError> {
        let mut val = ensure_not_null_slice::<Self>(typ, v)?;
        let cql = deser_cql_value(typ, &mut val).map_err(|err| {
            mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::GenericParseError(err))
        })?;
        Ok(cql)
    }
}

// Option represents nullability of CQL values:
// None corresponds to null,
// Some(val) to non-null values.
impl<'frame, T> DeserializeValue<'frame> for Option<T>
where
    T: DeserializeValue<'frame>,
{
    fn type_check(typ: &ColumnType) -> Result<(), TypeCheckError> {
        T::type_check(typ)
    }

    fn deserialize(
        typ: &'frame ColumnType,
        v: Option<FrameSlice<'frame>>,
    ) -> Result<Self, DeserializationError> {
        v.map(|_| T::deserialize(typ, v)).transpose()
    }
}

/// Values that may be empty or not.
///
/// In CQL, some types can have a special value of "empty", represented as
/// a serialized value of length 0. An example of this are integral types:
/// the "int" type can actually hold 2^32 + 1 possible values because of this
/// quirk. Note that this is distinct from being NULL.
///
/// Rust types that cannot represent an empty value (e.g. i32) should implement
/// this trait in order to be deserialized as [MaybeEmpty].
pub trait Emptiable {}

/// A value that may be empty or not.
///
/// `MaybeEmpty` was introduced to help support the quirk described in [Emptiable]
/// for Rust types which can't represent the empty, additional value.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub enum MaybeEmpty<T: Emptiable> {
    Empty,
    Value(T),
}

impl<'frame, T> DeserializeValue<'frame> for MaybeEmpty<T>
where
    T: DeserializeValue<'frame> + Emptiable,
{
    #[inline]
    fn type_check(typ: &ColumnType) -> Result<(), TypeCheckError> {
        <T as DeserializeValue<'frame>>::type_check(typ)
    }

    fn deserialize(
        typ: &'frame ColumnType,
        v: Option<FrameSlice<'frame>>,
    ) -> Result<Self, DeserializationError> {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        if val.is_empty() {
            Ok(MaybeEmpty::Empty)
        } else {
            let v = <T as DeserializeValue<'frame>>::deserialize(typ, v)?;
            Ok(MaybeEmpty::Value(v))
        }
    }
}

macro_rules! impl_strict_type {
    ($t:ty, [$($cql:ident)|+], $conv:expr $(, $l:lifetime)?) => {
        impl<$($l,)? 'frame> DeserializeValue<'frame> for $t
        where
            $('frame: $l)?
        {
            fn type_check(typ: &ColumnType) -> Result<(), TypeCheckError> {
                // TODO: Format the CQL type names in the same notation
                // that ScyllaDB/Cassandra uses internally and include them
                // in such form in the error message
                exact_type_check!(typ, $($cql),*);
                Ok(())
            }

            fn deserialize(
                typ: &'frame ColumnType,
                v: Option<FrameSlice<'frame>>,
            ) -> Result<Self, DeserializationError> {
                $conv(typ, v)
            }
        }
    };

    // Convenience pattern for omitting brackets if type-checking as single types.
    ($t:ty, $cql:ident, $conv:expr $(, $l:lifetime)?) => {
        impl_strict_type!($t, [$cql], $conv $(, $l)*);
    };
}

macro_rules! impl_emptiable_strict_type {
    ($t:ty, [$($cql:ident)|+], $conv:expr $(, $l:lifetime)?) => {
        impl<$($l,)?> Emptiable for $t {}

        impl_strict_type!($t, [$($cql)|*], $conv $(, $l)*);
    };

    // Convenience pattern for omitting brackets if type-checking as single types.
    ($t:ty, $cql:ident, $conv:expr $(, $l:lifetime)?) => {
        impl_emptiable_strict_type!($t, [$cql], $conv $(, $l)*);
    };

}

// fixed numeric types

macro_rules! impl_fixed_numeric_type {
    ($t:ty, [$($cql:ident)|+]) => {
        impl_emptiable_strict_type!(
            $t,
            [$($cql)|*],
            |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
                const SIZE: usize = std::mem::size_of::<$t>();
                let val = ensure_not_null_slice::<Self>(typ, v)?;
                let arr = ensure_exact_length::<Self, SIZE>(typ, val)?;
                Ok(<$t>::from_be_bytes(*arr))
            }
        );
    };

    // Convenience pattern for omitting brackets if type-checking as single types.
    ($t:ty, $cql:ident) => {
        impl_fixed_numeric_type!($t, [$cql]);
    };
}

impl_emptiable_strict_type!(
    bool,
    Boolean,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        let arr = ensure_exact_length::<Self, 1>(typ, val)?;
        Ok(arr[0] != 0x00)
    }
);

impl_fixed_numeric_type!(i8, TinyInt);
impl_fixed_numeric_type!(i16, SmallInt);
impl_fixed_numeric_type!(i32, Int);
impl_fixed_numeric_type!(i64, [BigInt | Counter]);
impl_fixed_numeric_type!(f32, Float);
impl_fixed_numeric_type!(f64, Double);

// other numeric types

impl_emptiable_strict_type!(
    CqlVarint,
    Varint,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        Ok(CqlVarint::from_signed_bytes_be_slice(val))
    }
);

#[cfg(feature = "num-bigint-03")]
impl_emptiable_strict_type!(num_bigint_03::BigInt, Varint, |typ: &'frame ColumnType,
                                                            v: Option<
    FrameSlice<'frame>,
>| {
    let val = ensure_not_null_slice::<Self>(typ, v)?;
    Ok(num_bigint_03::BigInt::from_signed_bytes_be(val))
});

#[cfg(feature = "num-bigint-04")]
impl_emptiable_strict_type!(num_bigint_04::BigInt, Varint, |typ: &'frame ColumnType,
                                                            v: Option<
    FrameSlice<'frame>,
>| {
    let val = ensure_not_null_slice::<Self>(typ, v)?;
    Ok(num_bigint_04::BigInt::from_signed_bytes_be(val))
});

impl_emptiable_strict_type!(
    CqlDecimal,
    Decimal,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let mut val = ensure_not_null_slice::<Self>(typ, v)?;
        let scale = types::read_int(&mut val).map_err(|err| {
            mk_deser_err::<Self>(
                typ,
                BuiltinDeserializationErrorKind::GenericParseError(err.into()),
            )
        })?;
        Ok(CqlDecimal::from_signed_be_bytes_slice_and_exponent(
            val, scale,
        ))
    }
);

#[cfg(feature = "bigdecimal-04")]
impl_emptiable_strict_type!(
    bigdecimal_04::BigDecimal,
    Decimal,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let mut val = ensure_not_null_slice::<Self>(typ, v)?;
        let scale = types::read_int(&mut val).map_err(|err| {
            mk_deser_err::<Self>(
                typ,
                BuiltinDeserializationErrorKind::GenericParseError(err.into()),
            )
        })? as i64;
        let int_value = bigdecimal_04::num_bigint::BigInt::from_signed_bytes_be(val);
        Ok(bigdecimal_04::BigDecimal::from((int_value, scale)))
    }
);

// blob

impl_strict_type!(
    &'a [u8],
    Blob,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        Ok(val)
    },
    'a
);
impl_strict_type!(
    Vec<u8>,
    Blob,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        Ok(val.to_vec())
    }
);
impl_strict_type!(
    Bytes,
    Blob,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_owned::<Self>(typ, v)?;
        Ok(val)
    }
);

// string

macro_rules! impl_string_type {
    ($t:ty, $conv:expr $(, $l:lifetime)?) => {
        impl_strict_type!(
            $t,
            [Ascii | Text],
            $conv
            $(, $l)?
        );
    }
}

fn check_ascii<T>(typ: &ColumnType, s: &[u8]) -> Result<(), DeserializationError> {
    if matches!(typ, ColumnType::Ascii) && !s.is_ascii() {
        return Err(mk_deser_err::<T>(
            typ,
            BuiltinDeserializationErrorKind::ExpectedAscii,
        ));
    }
    Ok(())
}

impl_string_type!(
    &'a str,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        check_ascii::<&str>(typ, val)?;
        let s = std::str::from_utf8(val).map_err(|err| {
            mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::InvalidUtf8(err))
        })?;
        Ok(s)
    },
    'a
);
impl_string_type!(
    String,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        check_ascii::<String>(typ, val)?;
        let s = std::str::from_utf8(val).map_err(|err| {
            mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::InvalidUtf8(err))
        })?;
        Ok(s.to_string())
    }
);

// TODO: Consider support for deserialization of string::String<Bytes>

// counter

impl_strict_type!(
    Counter,
    Counter,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        let arr = ensure_exact_length::<Self, 8>(typ, val)?;
        let counter = i64::from_be_bytes(*arr);
        Ok(Counter(counter))
    }
);

// date and time types

// duration
impl_strict_type!(
    CqlDuration,
    Duration,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let mut val = ensure_not_null_slice::<Self>(typ, v)?;

        macro_rules! mk_err {
            ($err: expr) => {
                mk_deser_err::<Self>(typ, $err)
            };
        }

        let months_i64 = types::vint_decode(&mut val).map_err(|err| {
            mk_err!(BuiltinDeserializationErrorKind::GenericParseError(
                err.into()
            ))
        })?;
        let months = i32::try_from(months_i64)
            .map_err(|_| mk_err!(BuiltinDeserializationErrorKind::ValueOverflow))?;

        let days_i64 = types::vint_decode(&mut val).map_err(|err| {
            mk_err!(BuiltinDeserializationErrorKind::GenericParseError(
                err.into()
            ))
        })?;
        let days = i32::try_from(days_i64)
            .map_err(|_| mk_err!(BuiltinDeserializationErrorKind::ValueOverflow))?;

        let nanoseconds = types::vint_decode(&mut val).map_err(|err| {
            mk_err!(BuiltinDeserializationErrorKind::GenericParseError(
                err.into()
            ))
        })?;

        Ok(CqlDuration {
            months,
            days,
            nanoseconds,
        })
    }
);

impl_emptiable_strict_type!(
    CqlDate,
    Date,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let val = ensure_not_null_slice::<Self>(typ, v)?;
        let arr = ensure_exact_length::<Self, 4>(typ, val)?;
        let days = u32::from_be_bytes(*arr);
        Ok(CqlDate(days))
    }
);

#[cfg(any(feature = "chrono", feature = "time"))]
fn get_days_since_epoch_from_date_column<T>(
    typ: &ColumnType,
    v: Option<FrameSlice<'_>>,
) -> Result<i64, DeserializationError> {
    let val = ensure_not_null_slice::<T>(typ, v)?;
    let arr = ensure_exact_length::<T, 4>(typ, val)?;
    let days = u32::from_be_bytes(*arr);
    let days_since_epoch = days as i64 - (1i64 << 31);
    Ok(days_since_epoch)
}

#[cfg(feature = "chrono")]
impl_emptiable_strict_type!(
    chrono::NaiveDate,
    Date,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let fail = || mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::ValueOverflow);
        let days_since_epoch =
            chrono::Duration::try_days(get_days_since_epoch_from_date_column::<Self>(typ, v)?)
                .ok_or_else(fail)?;
        chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .checked_add_signed(days_since_epoch)
            .ok_or_else(fail)
    }
);

#[cfg(feature = "time")]
impl_emptiable_strict_type!(
    time::Date,
    Date,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let days_since_epoch =
            time::Duration::days(get_days_since_epoch_from_date_column::<Self>(typ, v)?);
        time::Date::from_calendar_date(1970, time::Month::January, 1)
            .unwrap()
            .checked_add(days_since_epoch)
            .ok_or_else(|| {
                mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::ValueOverflow)
            })
    }
);

fn get_nanos_from_time_column<T>(
    typ: &ColumnType,
    v: Option<FrameSlice<'_>>,
) -> Result<i64, DeserializationError> {
    let val = ensure_not_null_slice::<T>(typ, v)?;
    let arr = ensure_exact_length::<T, 8>(typ, val)?;
    let nanoseconds = i64::from_be_bytes(*arr);

    // Valid values are in the range 0 to 86399999999999
    if !(0..=86399999999999).contains(&nanoseconds) {
        return Err(mk_deser_err::<T>(
            typ,
            BuiltinDeserializationErrorKind::ValueOverflow,
        ));
    }

    Ok(nanoseconds)
}

impl_emptiable_strict_type!(
    CqlTime,
    Time,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let nanoseconds = get_nanos_from_time_column::<Self>(typ, v)?;

        Ok(CqlTime(nanoseconds))
    }
);

#[cfg(feature = "chrono")]
impl_emptiable_strict_type!(
    chrono::NaiveTime,
    Time,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let nanoseconds = get_nanos_from_time_column::<chrono::NaiveTime>(typ, v)?;

        let naive_time: chrono::NaiveTime = CqlTime(nanoseconds).try_into().map_err(|_| {
            mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::ValueOverflow)
        })?;
        Ok(naive_time)
    }
);

#[cfg(feature = "time")]
impl_emptiable_strict_type!(
    time::Time,
    Time,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let nanoseconds = get_nanos_from_time_column::<time::Time>(typ, v)?;

        let time: time::Time = CqlTime(nanoseconds).try_into().map_err(|_| {
            mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::ValueOverflow)
        })?;
        Ok(time)
    }
);

fn get_millis_from_timestamp_column<T>(
    typ: &ColumnType,
    v: Option<FrameSlice<'_>>,
) -> Result<i64, DeserializationError> {
    let val = ensure_not_null_slice::<T>(typ, v)?;
    let arr = ensure_exact_length::<T, 8>(typ, val)?;
    let millis = i64::from_be_bytes(*arr);

    Ok(millis)
}

impl_emptiable_strict_type!(
    CqlTimestamp,
    Timestamp,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let millis = get_millis_from_timestamp_column::<Self>(typ, v)?;
        Ok(CqlTimestamp(millis))
    }
);

#[cfg(feature = "chrono")]
impl_emptiable_strict_type!(
    chrono::DateTime<chrono::Utc>,
    Timestamp,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        use chrono::TimeZone as _;

        let millis = get_millis_from_timestamp_column::<Self>(typ, v)?;
        match chrono::Utc.timestamp_millis_opt(millis) {
            chrono::LocalResult::Single(datetime) => Ok(datetime),
            _ => Err(mk_deser_err::<Self>(
                typ,
                BuiltinDeserializationErrorKind::ValueOverflow,
            )),
        }
    }
);

#[cfg(feature = "time")]
impl_emptiable_strict_type!(
    time::OffsetDateTime,
    Timestamp,
    |typ: &'frame ColumnType, v: Option<FrameSlice<'frame>>| {
        let millis = get_millis_from_timestamp_column::<Self>(typ, v)?;
        time::OffsetDateTime::from_unix_timestamp_nanos(millis as i128 * 1_000_000)
            .map_err(|_| mk_deser_err::<Self>(typ, BuiltinDeserializationErrorKind::ValueOverflow))
    }
);

// Utilities

fn ensure_not_null_frame_slice<'frame, T>(
    typ: &ColumnType,
    v: Option<FrameSlice<'frame>>,
) -> Result<FrameSlice<'frame>, DeserializationError> {
    v.ok_or_else(|| mk_deser_err::<T>(typ, BuiltinDeserializationErrorKind::ExpectedNonNull))
}

fn ensure_not_null_slice<'frame, T>(
    typ: &ColumnType,
    v: Option<FrameSlice<'frame>>,
) -> Result<&'frame [u8], DeserializationError> {
    ensure_not_null_frame_slice::<T>(typ, v).map(|frame_slice| frame_slice.as_slice())
}

fn ensure_not_null_owned<T>(
    typ: &ColumnType,
    v: Option<FrameSlice>,
) -> Result<Bytes, DeserializationError> {
    ensure_not_null_frame_slice::<T>(typ, v).map(|frame_slice| frame_slice.to_bytes())
}

fn ensure_exact_length<'frame, T, const SIZE: usize>(
    typ: &ColumnType,
    v: &'frame [u8],
) -> Result<&'frame [u8; SIZE], DeserializationError> {
    v.try_into().map_err(|_| {
        mk_deser_err::<T>(
            typ,
            BuiltinDeserializationErrorKind::ByteLengthMismatch {
                expected: SIZE,
                got: v.len(),
            },
        )
    })
}

// Error facilities

/// Type checking of one of the built-in types failed.
#[derive(Debug, Error, Clone)]
#[error("Failed to type check Rust type {rust_name} against CQL type {cql_type:?}: {kind}")]
pub struct BuiltinTypeCheckError {
    /// Name of the Rust type being deserialized.
    pub rust_name: &'static str,

    /// The CQL type that the Rust type was being deserialized from.
    pub cql_type: ColumnType,

    /// Detailed information about the failure.
    pub kind: BuiltinTypeCheckErrorKind,
}

fn mk_typck_err<T>(
    cql_type: &ColumnType,
    kind: impl Into<BuiltinTypeCheckErrorKind>,
) -> TypeCheckError {
    mk_typck_err_named(std::any::type_name::<T>(), cql_type, kind)
}

fn mk_typck_err_named(
    name: &'static str,
    cql_type: &ColumnType,
    kind: impl Into<BuiltinTypeCheckErrorKind>,
) -> TypeCheckError {
    TypeCheckError::new(BuiltinTypeCheckError {
        rust_name: name,
        cql_type: cql_type.clone(),
        kind: kind.into(),
    })
}

macro_rules! exact_type_check {
    ($typ:ident, $($cql:tt),*) => {
        match $typ {
            $(ColumnType::$cql)|* => {},
            _ => return Err(mk_typck_err::<Self>(
                $typ,
                BuiltinTypeCheckErrorKind::MismatchedType {
                    expected: &[$(ColumnType::$cql),*],
                }
            ))
        }
    };
}
use exact_type_check;

/// Describes why type checking some of the built-in types failed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BuiltinTypeCheckErrorKind {
    /// Expected one from a list of particular types.
    MismatchedType {
        /// The list of types that the Rust type can deserialize from.
        expected: &'static [ColumnType],
    },
}

impl Display for BuiltinTypeCheckErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuiltinTypeCheckErrorKind::MismatchedType { expected } => {
                write!(f, "expected one of the CQL types: {expected:?}")
            }
        }
    }
}

/// Deserialization of one of the built-in types failed.
#[derive(Debug, Error)]
#[error("Failed to deserialize Rust type {rust_name} from CQL type {cql_type:?}: {kind}")]
pub struct BuiltinDeserializationError {
    /// Name of the Rust type being deserialized.
    pub rust_name: &'static str,

    /// The CQL type that the Rust type was being deserialized from.
    pub cql_type: ColumnType,

    /// Detailed information about the failure.
    pub kind: BuiltinDeserializationErrorKind,
}

fn mk_deser_err<T>(
    cql_type: &ColumnType,
    kind: impl Into<BuiltinDeserializationErrorKind>,
) -> DeserializationError {
    mk_deser_err_named(std::any::type_name::<T>(), cql_type, kind)
}

fn mk_deser_err_named(
    name: &'static str,
    cql_type: &ColumnType,
    kind: impl Into<BuiltinDeserializationErrorKind>,
) -> DeserializationError {
    DeserializationError::new(BuiltinDeserializationError {
        rust_name: name,
        cql_type: cql_type.clone(),
        kind: kind.into(),
    })
}

/// Describes why deserialization of some of the built-in types failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum BuiltinDeserializationErrorKind {
    /// A generic deserialization failure - legacy error type.
    GenericParseError(ParseError),

    /// Expected non-null value, got null.
    ExpectedNonNull,

    /// The length of read value in bytes is different than expected for the Rust type.
    ByteLengthMismatch { expected: usize, got: usize },

    /// Expected valid ASCII string.
    ExpectedAscii,

    /// Invalid UTF-8 string.
    InvalidUtf8(std::str::Utf8Error),

    /// The read value is out of range supported by the Rust type.
    // TODO: consider storing additional info here (what exactly did not fit and why)
    ValueOverflow,
}

impl Display for BuiltinDeserializationErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuiltinDeserializationErrorKind::GenericParseError(err) => err.fmt(f),
            BuiltinDeserializationErrorKind::ExpectedNonNull => {
                f.write_str("expected a non-null value, got null")
            }
            BuiltinDeserializationErrorKind::ByteLengthMismatch { expected, got } => write!(
                f,
                "the CQL type requires {} bytes, but got {}",
                expected, got,
            ),
            BuiltinDeserializationErrorKind::ExpectedAscii => {
                f.write_str("expected a valid ASCII string")
            }
            BuiltinDeserializationErrorKind::InvalidUtf8(err) => err.fmt(f),
            BuiltinDeserializationErrorKind::ValueOverflow => {
                // TODO: consider storing Arc<dyn Display/Debug> of the offending value
                // inside this variant for debug purposes.
                f.write_str("read value is out of representable range")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::{BufMut, Bytes, BytesMut};

    use std::fmt::Debug;

    use crate::frame::response::cql_to_rust::FromCqlVal;
    use crate::frame::response::result::{deser_cql_value, ColumnType, CqlValue};
    use crate::frame::types;
    use crate::frame::value::{
        Counter, CqlDate, CqlDecimal, CqlDuration, CqlTime, CqlTimestamp, CqlVarint,
    };
    use crate::types::deserialize::{DeserializationError, FrameSlice};
    use crate::types::serialize::value::SerializeValue;
    use crate::types::serialize::CellWriter;

    use super::{mk_deser_err, BuiltinDeserializationErrorKind, DeserializeValue};

    #[test]
    fn test_deserialize_bytes() {
        const ORIGINAL_BYTES: &[u8] = &[1, 5, 2, 4, 3];

        let bytes = make_bytes(ORIGINAL_BYTES);

        let decoded_slice = deserialize::<&[u8]>(&ColumnType::Blob, &bytes).unwrap();
        let decoded_vec = deserialize::<Vec<u8>>(&ColumnType::Blob, &bytes).unwrap();
        let decoded_bytes = deserialize::<Bytes>(&ColumnType::Blob, &bytes).unwrap();

        assert_eq!(decoded_slice, ORIGINAL_BYTES);
        assert_eq!(decoded_vec, ORIGINAL_BYTES);
        assert_eq!(decoded_bytes, ORIGINAL_BYTES);
    }

    #[test]
    fn test_deserialize_ascii() {
        const ASCII_TEXT: &str = "The quick brown fox jumps over the lazy dog";

        let ascii = make_bytes(ASCII_TEXT.as_bytes());

        let decoded_ascii_str = deserialize::<&str>(&ColumnType::Ascii, &ascii).unwrap();
        let decoded_ascii_string = deserialize::<String>(&ColumnType::Ascii, &ascii).unwrap();
        let decoded_text_str = deserialize::<&str>(&ColumnType::Text, &ascii).unwrap();
        let decoded_text_string = deserialize::<String>(&ColumnType::Text, &ascii).unwrap();

        assert_eq!(decoded_ascii_str, ASCII_TEXT);
        assert_eq!(decoded_ascii_string, ASCII_TEXT);
        assert_eq!(decoded_text_str, ASCII_TEXT);
        assert_eq!(decoded_text_string, ASCII_TEXT);
    }

    #[test]
    fn test_deserialize_text() {
        const UNICODE_TEXT: &str = "Zażółć gęślą jaźń";

        let unicode = make_bytes(UNICODE_TEXT.as_bytes());

        // Should fail because it's not an ASCII string
        deserialize::<&str>(&ColumnType::Ascii, &unicode).unwrap_err();
        deserialize::<String>(&ColumnType::Ascii, &unicode).unwrap_err();

        let decoded_text_str = deserialize::<&str>(&ColumnType::Text, &unicode).unwrap();
        let decoded_text_string = deserialize::<String>(&ColumnType::Text, &unicode).unwrap();
        assert_eq!(decoded_text_str, UNICODE_TEXT);
        assert_eq!(decoded_text_string, UNICODE_TEXT);
    }

    #[test]
    fn test_integral() {
        let tinyint = make_bytes(&[0x01]);
        let decoded_tinyint = deserialize::<i8>(&ColumnType::TinyInt, &tinyint).unwrap();
        assert_eq!(decoded_tinyint, 0x01);

        let smallint = make_bytes(&[0x01, 0x02]);
        let decoded_smallint = deserialize::<i16>(&ColumnType::SmallInt, &smallint).unwrap();
        assert_eq!(decoded_smallint, 0x0102);

        let int = make_bytes(&[0x01, 0x02, 0x03, 0x04]);
        let decoded_int = deserialize::<i32>(&ColumnType::Int, &int).unwrap();
        assert_eq!(decoded_int, 0x01020304);

        let bigint = make_bytes(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let decoded_bigint = deserialize::<i64>(&ColumnType::BigInt, &bigint).unwrap();
        assert_eq!(decoded_bigint, 0x0102030405060708);
    }

    #[test]
    fn test_bool() {
        for boolean in [true, false] {
            let boolean_bytes = make_bytes(&[boolean as u8]);
            let decoded_bool = deserialize::<bool>(&ColumnType::Boolean, &boolean_bytes).unwrap();
            assert_eq!(decoded_bool, boolean);
        }
    }

    #[test]
    fn test_floating_point() {
        let float = make_bytes(&[63, 0, 0, 0]);
        let decoded_float = deserialize::<f32>(&ColumnType::Float, &float).unwrap();
        assert_eq!(decoded_float, 0.5);

        let double = make_bytes(&[64, 0, 0, 0, 0, 0, 0, 0]);
        let decoded_double = deserialize::<f64>(&ColumnType::Double, &double).unwrap();
        assert_eq!(decoded_double, 2.0);
    }

    #[test]
    fn test_from_cql_value_compatibility() {
        // This test should have a sub-case for each type
        // that implements FromCqlValue

        // fixed size integers
        for i in 0..7 {
            let v: i8 = 1 << i;
            compat_check::<i8>(&ColumnType::TinyInt, make_bytes(&v.to_be_bytes()));
            compat_check::<i8>(&ColumnType::TinyInt, make_bytes(&(-v).to_be_bytes()));
        }
        for i in 0..15 {
            let v: i16 = 1 << i;
            compat_check::<i16>(&ColumnType::SmallInt, make_bytes(&v.to_be_bytes()));
            compat_check::<i16>(&ColumnType::SmallInt, make_bytes(&(-v).to_be_bytes()));
        }
        for i in 0..31 {
            let v: i32 = 1 << i;
            compat_check::<i32>(&ColumnType::Int, make_bytes(&v.to_be_bytes()));
            compat_check::<i32>(&ColumnType::Int, make_bytes(&(-v).to_be_bytes()));
        }
        for i in 0..63 {
            let v: i64 = 1 << i;
            compat_check::<i64>(&ColumnType::BigInt, make_bytes(&v.to_be_bytes()));
            compat_check::<i64>(&ColumnType::BigInt, make_bytes(&(-v).to_be_bytes()));
        }

        // bool
        compat_check::<bool>(&ColumnType::Boolean, make_bytes(&[0]));
        compat_check::<bool>(&ColumnType::Boolean, make_bytes(&[1]));

        // fixed size floating point types
        compat_check::<f32>(&ColumnType::Float, make_bytes(&123f32.to_be_bytes()));
        compat_check::<f32>(&ColumnType::Float, make_bytes(&(-123f32).to_be_bytes()));
        compat_check::<f64>(&ColumnType::Double, make_bytes(&123f64.to_be_bytes()));
        compat_check::<f64>(&ColumnType::Double, make_bytes(&(-123f64).to_be_bytes()));

        // big integers
        const PI_STR: &[u8] = b"3.1415926535897932384626433832795028841971693993751058209749445923";
        let num1 = &PI_STR[2..];
        let num2 = [b'-']
            .into_iter()
            .chain(PI_STR[2..].iter().copied())
            .collect::<Vec<_>>();
        let num3 = &b"0"[..];

        // native - CqlVarint
        {
            let num1 = CqlVarint::from_signed_bytes_be_slice(num1);
            let num2 = CqlVarint::from_signed_bytes_be_slice(&num2);
            let num3 = CqlVarint::from_signed_bytes_be_slice(num3);
            compat_check_serialized::<CqlVarint>(&ColumnType::Varint, &num1);
            compat_check_serialized::<CqlVarint>(&ColumnType::Varint, &num2);
            compat_check_serialized::<CqlVarint>(&ColumnType::Varint, &num3);
        }

        #[cfg(feature = "num-bigint-03")]
        {
            use num_bigint_03::BigInt;

            let num1 = BigInt::parse_bytes(num1, 10).unwrap();
            let num2 = BigInt::parse_bytes(&num2, 10).unwrap();
            let num3 = BigInt::parse_bytes(num3, 10).unwrap();
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num1);
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num2);
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num3);
        }

        #[cfg(feature = "num-bigint-04")]
        {
            use num_bigint_04::BigInt;

            let num1 = BigInt::parse_bytes(num1, 10).unwrap();
            let num2 = BigInt::parse_bytes(&num2, 10).unwrap();
            let num3 = BigInt::parse_bytes(num3, 10).unwrap();
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num1);
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num2);
            compat_check_serialized::<BigInt>(&ColumnType::Varint, &num3);
        }

        // big decimals
        {
            let scale1 = 0;
            let scale2 = -42;
            let scale3 = 2137;
            let num1 = CqlDecimal::from_signed_be_bytes_slice_and_exponent(num1, scale1);
            let num2 = CqlDecimal::from_signed_be_bytes_and_exponent(num2, scale2);
            let num3 = CqlDecimal::from_signed_be_bytes_slice_and_exponent(num3, scale3);
            compat_check_serialized::<CqlDecimal>(&ColumnType::Decimal, &num1);
            compat_check_serialized::<CqlDecimal>(&ColumnType::Decimal, &num2);
            compat_check_serialized::<CqlDecimal>(&ColumnType::Decimal, &num3);
        }

        // native - CqlDecimal

        #[cfg(feature = "bigdecimal-04")]
        {
            use bigdecimal_04::BigDecimal;

            let num1 = PI_STR.to_vec();
            let num2 = vec![b'-']
                .into_iter()
                .chain(PI_STR.iter().copied())
                .collect::<Vec<_>>();
            let num3 = b"0.0".to_vec();

            let num1 = BigDecimal::parse_bytes(&num1, 10).unwrap();
            let num2 = BigDecimal::parse_bytes(&num2, 10).unwrap();
            let num3 = BigDecimal::parse_bytes(&num3, 10).unwrap();
            compat_check_serialized::<BigDecimal>(&ColumnType::Decimal, &num1);
            compat_check_serialized::<BigDecimal>(&ColumnType::Decimal, &num2);
            compat_check_serialized::<BigDecimal>(&ColumnType::Decimal, &num3);
        }

        // blob
        compat_check::<Vec<u8>>(&ColumnType::Blob, make_bytes(&[]));
        compat_check::<Vec<u8>>(&ColumnType::Blob, make_bytes(&[1, 9, 2, 8, 3, 7, 4, 6, 5]));

        // text types
        for typ in &[ColumnType::Ascii, ColumnType::Text] {
            compat_check::<String>(typ, make_bytes("".as_bytes()));
            compat_check::<String>(typ, make_bytes("foo".as_bytes()));
            compat_check::<String>(typ, make_bytes("superfragilisticexpialidocious".as_bytes()));
        }

        // counters
        for i in 0..63 {
            let v: i64 = 1 << i;
            compat_check::<Counter>(&ColumnType::Counter, make_bytes(&v.to_be_bytes()));
        }

        // duration
        let duration1 = CqlDuration {
            days: 123,
            months: 456,
            nanoseconds: 789,
        };
        let duration2 = CqlDuration {
            days: 987,
            months: 654,
            nanoseconds: 321,
        };
        compat_check_serialized::<CqlDuration>(&ColumnType::Duration, &duration1);
        compat_check_serialized::<CqlDuration>(&ColumnType::Duration, &duration2);

        // date
        let date1 = (2u32.pow(31)).to_be_bytes();
        let date2 = (2u32.pow(31) - 30).to_be_bytes();
        let date3 = (2u32.pow(31) + 30).to_be_bytes();

        compat_check::<CqlDate>(&ColumnType::Date, make_bytes(&date1));
        compat_check::<CqlDate>(&ColumnType::Date, make_bytes(&date2));
        compat_check::<CqlDate>(&ColumnType::Date, make_bytes(&date3));

        #[cfg(feature = "chrono")]
        {
            compat_check::<chrono::NaiveDate>(&ColumnType::Date, make_bytes(&date1));
            compat_check::<chrono::NaiveDate>(&ColumnType::Date, make_bytes(&date2));
            compat_check::<chrono::NaiveDate>(&ColumnType::Date, make_bytes(&date3));
        }

        #[cfg(feature = "time")]
        {
            compat_check::<time::Date>(&ColumnType::Date, make_bytes(&date1));
            compat_check::<time::Date>(&ColumnType::Date, make_bytes(&date2));
            compat_check::<time::Date>(&ColumnType::Date, make_bytes(&date3));
        }

        // time
        let time1 = CqlTime(0);
        let time2 = CqlTime(123456789);
        let time3 = CqlTime(86399999999999); // maximum allowed

        compat_check_serialized::<CqlTime>(&ColumnType::Time, &time1);
        compat_check_serialized::<CqlTime>(&ColumnType::Time, &time2);
        compat_check_serialized::<CqlTime>(&ColumnType::Time, &time3);

        #[cfg(feature = "chrono")]
        {
            compat_check_serialized::<chrono::NaiveTime>(&ColumnType::Time, &time1);
            compat_check_serialized::<chrono::NaiveTime>(&ColumnType::Time, &time2);
            compat_check_serialized::<chrono::NaiveTime>(&ColumnType::Time, &time3);
        }

        #[cfg(feature = "time")]
        {
            compat_check_serialized::<time::Time>(&ColumnType::Time, &time1);
            compat_check_serialized::<time::Time>(&ColumnType::Time, &time2);
            compat_check_serialized::<time::Time>(&ColumnType::Time, &time3);
        }

        // timestamp
        let timestamp1 = CqlTimestamp(0);
        let timestamp2 = CqlTimestamp(123456789);
        let timestamp3 = CqlTimestamp(98765432123456);

        compat_check_serialized::<CqlTimestamp>(&ColumnType::Timestamp, &timestamp1);
        compat_check_serialized::<CqlTimestamp>(&ColumnType::Timestamp, &timestamp2);
        compat_check_serialized::<CqlTimestamp>(&ColumnType::Timestamp, &timestamp3);

        #[cfg(feature = "chrono")]
        {
            compat_check_serialized::<chrono::DateTime<chrono::Utc>>(
                &ColumnType::Timestamp,
                &timestamp1,
            );
            compat_check_serialized::<chrono::DateTime<chrono::Utc>>(
                &ColumnType::Timestamp,
                &timestamp2,
            );
            compat_check_serialized::<chrono::DateTime<chrono::Utc>>(
                &ColumnType::Timestamp,
                &timestamp3,
            );
        }

        #[cfg(feature = "time")]
        {
            compat_check_serialized::<time::OffsetDateTime>(&ColumnType::Timestamp, &timestamp1);
            compat_check_serialized::<time::OffsetDateTime>(&ColumnType::Timestamp, &timestamp2);
            compat_check_serialized::<time::OffsetDateTime>(&ColumnType::Timestamp, &timestamp3);
        }
    }

    // Checks that both new and old serialization framework
    // produces the same results in this case
    fn compat_check<T>(typ: &ColumnType, raw: Bytes)
    where
        T: for<'f> DeserializeValue<'f>,
        T: FromCqlVal<Option<CqlValue>>,
        T: Debug + PartialEq,
    {
        let mut slice = raw.as_ref();
        let mut cell = types::read_bytes_opt(&mut slice).unwrap();
        let old = T::from_cql(
            cell.as_mut()
                .map(|c| deser_cql_value(typ, c))
                .transpose()
                .unwrap(),
        )
        .unwrap();
        let new = deserialize::<T>(typ, &raw).unwrap();
        assert_eq!(old, new);
    }

    fn compat_check_serialized<T>(typ: &ColumnType, val: &dyn SerializeValue)
    where
        T: for<'f> DeserializeValue<'f>,
        T: FromCqlVal<Option<CqlValue>>,
        T: Debug + PartialEq,
    {
        let raw = serialize(typ, val);
        compat_check::<T>(typ, raw);
    }

    fn deserialize<'frame, T>(
        typ: &'frame ColumnType,
        bytes: &'frame Bytes,
    ) -> Result<T, DeserializationError>
    where
        T: DeserializeValue<'frame>,
    {
        <T as DeserializeValue<'frame>>::type_check(typ)
            .map_err(|typecheck_err| DeserializationError(typecheck_err.0))?;
        let mut frame_slice = FrameSlice::new(bytes);
        let value = frame_slice.read_cql_bytes().map_err(|err| {
            mk_deser_err::<T>(typ, BuiltinDeserializationErrorKind::GenericParseError(err))
        })?;
        <T as DeserializeValue<'frame>>::deserialize(typ, value)
    }

    fn make_bytes(cell: &[u8]) -> Bytes {
        let mut b = BytesMut::new();
        append_bytes(&mut b, cell);
        b.freeze()
    }

    fn serialize(typ: &ColumnType, value: &dyn SerializeValue) -> Bytes {
        let mut bytes = Bytes::new();
        serialize_to_buf(typ, value, &mut bytes);
        bytes
    }

    fn serialize_to_buf(typ: &ColumnType, value: &dyn SerializeValue, buf: &mut Bytes) {
        let mut v = Vec::new();
        let writer = CellWriter::new(&mut v);
        value.serialize(typ, writer).unwrap();
        *buf = v.into();
    }

    fn append_bytes(b: &mut impl BufMut, cell: &[u8]) {
        b.put_i32(cell.len() as i32);
        b.put_slice(cell);
    }
}
