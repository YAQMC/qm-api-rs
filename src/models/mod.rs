//! 模型模块.

pub mod album;
pub mod base;
pub mod comment;
pub mod helper;
pub mod login;
pub mod lyric;
pub mod mv;
pub mod private_message;
pub mod recommend;
pub mod request;
pub mod search;
pub mod singer;
pub mod song;
pub mod songlist;
pub mod top;
pub mod user;

pub use base::{Album, File, MV, Pay, Singer, Song, SongList};
pub use request::Credential;

/// 按提取策略反序列化 JSONPath 字段 (由 `jsonpath_model!` 内部使用).
#[doc(hidden)]
#[macro_export]
macro_rules! jsonpath_extract {
    (strict, $ty:ty, $v:expr, $p:expr) => {
        $crate::jsonpath::extract_strict::<$ty>($v, $p).map_err(::serde::de::Error::custom)?
    };
    (optional, $ty:ty, $v:expr, $p:expr) => {
        $crate::jsonpath::extract_optional::<$ty>($v, $p)
    };
    (default, $ty:ty, $v:expr, $p:expr) => {
        $crate::jsonpath::extract_typed::<$ty>($v, $p)
    };
}

/// 定义通过 JSONPath 提取字段的响应模型.
///
/// 每个字段声明 `JSONPath` 表达式与类型; 提取策略三选一:
/// - `field: expr => Ty`            —— lenient, 缺失时用默认值 (仅用于明确可容忍的字段);
/// - `field: expr => optional(Ty)`  —— 缺失时得到 `Option<Ty>`;
/// - `field: expr => strict(Ty)`    —— 缺失/类型不符即反序列化报错 (暴露 schema drift).
#[macro_export]
macro_rules! jsonpath_model {
    ($name:ident { $( $(#[$meta:meta])* $field:ident: $path:expr => $ext:ident($ty:ty) ),* $(,)? }) => {
        #[derive(Debug, Clone, Default)]
        #[allow(non_snake_case)]
        pub struct $name {
            $( $(#[$meta])* pub $field: $ty ),*
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                de: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let raw = <::serde_json::Value as ::serde::Deserialize>::deserialize(de)?;
                Ok($name {
                    $($field: $crate::jsonpath_extract!($ext, $ty, &raw, $path)),*
                })
            }
        }
    };
    ($name:ident { $( $(#[$meta:meta])* $field:ident: $path:expr => $ty:ty ),* $(,)? }) => {
        #[derive(Debug, Clone, Default)]
        #[allow(non_snake_case)]
        pub struct $name {
            $( $(#[$meta])* pub $field: $ty ),*
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                de: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let raw = <::serde_json::Value as ::serde::Deserialize>::deserialize(de)?;
                Ok($name {
                    $($field: $crate::jsonpath_extract!(default, $ty, &raw, $path)),*
                })
            }
        }
    };
}

/// 为任意类型实现 `From<serde_json::Value>` 并返回默认值兜底.
pub fn from_value<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Option<T> {
    if value.is_null() {
        None
    } else {
        serde_json::from_value(value.clone()).ok()
    }
}

/// 字段级反序列化辅助 (对应 Python 端 `models/_validator.py`).
pub mod de {
    use serde::de::Deserializer;
    use serde::Deserialize;
    use serde_json::Value;

    /// 将 `null` 规整为该类型的默认值 (如空列表 / 空字典 / 空字符串).
    pub fn null_as_default<'de, T, D>(de: D) -> Result<T, D::Error>
    where
        T: Deserialize<'de> + Default,
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(de)?;
        if value.is_null() {
            Ok(T::default())
        } else {
            T::deserialize(value).map_err(serde::de::Error::custom)
        }
    }

    /// 将 `null` 或 `0` 规整为空字符串 (对应 `NoneOrZeroToEmptyStr`).
    pub fn str_or_zero<'de, D>(de: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(de)?;
        match value {
            Value::Null => Ok(String::new()),
            Value::Number(n) => {
                if n.as_i64() == Some(0) || n.as_u64() == Some(0) {
                    Ok(String::new())
                } else {
                    Ok(n.to_string())
                }
            }
            other => Ok(other.as_str().unwrap_or("").to_string()),
        }
    }
}
