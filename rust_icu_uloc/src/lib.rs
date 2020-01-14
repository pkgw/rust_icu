// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use {
    rust_icu_common as common,
    rust_icu_sys::versioned_function,
    rust_icu_sys::*,
    rust_icu_uenum::Enumeration,
    std::{
        convert::{From, TryFrom, TryInto},
        ffi,
        os::raw,
    },
};

/// Maximum length of locale supported by uloc.h.
/// See `ULOC_FULLNAME_CAPACITY`.
const LOCALE_CAPACITY: usize = 158;

/// A representation of a Unicode locale.
///
/// For the time being, only basic conversion and methods are in fact implemented.
#[derive(Debug, Eq, PartialEq)]
pub struct ULoc {
    // A locale's representation in C is really just a string.
    repr: String,
}

impl TryFrom<&str> for ULoc {
    type Error = common::Error;
    /// Creates a new ULoc from a string slice.
    ///
    /// The creation wil fail if the locale is nonexistent.
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let s = String::from(s);
        ULoc { repr: s }.canonicalize()
    }
}

impl TryFrom<&ffi::CStr> for ULoc {
    type Error = common::Error;

    /// Creates a new `ULoc` from a borrowed C string.
    fn try_from(s: &ffi::CStr) -> Result<Self, Self::Error> {
        let repr = s
            .to_str()
            .map_err(|_| common::Error::string_with_interior_nul())?;
        ULoc {
            repr: String::from(repr),
        }
        .canonicalize()
    }
}

/// Generates a method to wrap ICU4C `uloc` methods that require a resizable output string buffer.
///
/// The various `uloc` methods of this type have inconsistent signature patterns, with some putting
/// all their input arguments _before_ the `buffer` and its `capacity`, and some splitting the input
/// arguments.
///
/// Therefore, the macro supports input arguments in both positions.
///
/// For an invocation of the form
/// ```
/// buffered_string_method_with_retry!(
///     my_method,
///     BUFFER_CAPACITY,
///     [before_arg_a: before_type_a, before_arg_b: before_type_b,],
///     [after_arg_a: after_type_a, after_arg_b: after_type_b,]
/// );   
/// ```
/// the generated method has a signature of the form
/// ```
/// fn my_method(
///     uloc_method: unsafe extern "C" fn(
///         before_type_a,
///         before_type_b,
///         *mut raw::c_char,
///         i32,
///         after_type_a,
///         after_type_b,
///         *mut UErrorCode,
///     ) -> i32,
///     before_arg_a: before_type_a,
///     before_arg_b: before_type_b,
///     after_arg_a: after_type_a,
///     after_arg_b: after_type_b
/// ) -> Result<String, common::Error> {}
/// ```
macro_rules! buffered_string_method_with_retry {

    ($method_name:ident, $buffer_capacity:expr,
     [$($before_arg:ident: $before_arg_type:ty,)*],
     [$($after_arg:ident: $after_arg_type:ty,)*]) => {
        fn $method_name(
            uloc_method: unsafe extern "C" fn(
                $($before_arg_type,)*
                *mut raw::c_char,
                i32,
                $($after_arg_type,)*
                *mut UErrorCode,
            ) -> i32,
            $($before_arg: $before_arg_type,)*
            $($after_arg: $after_arg_type,)*
        ) -> Result<String, common::Error> {
            let mut status = common::Error::OK_CODE;
            let mut buf: Vec<u8> = vec![0; $buffer_capacity];

            // Requires that any pointers that are passed in are valid.
            let full_len: i32 = unsafe {
                assert!(common::Error::is_ok(status));
                uloc_method(
                    $($before_arg,)*
                    buf.as_mut_ptr() as *mut raw::c_char,
                    $buffer_capacity as i32,
                    $($after_arg,)*
                    &mut status,
                )
            };

            // `uloc` methods are inconsistent in whether they silently truncate the output or treat
            // the overflow as an error, so we need to check both cases.
            if status == UErrorCode::U_BUFFER_OVERFLOW_ERROR ||
               (common::Error::is_ok(status) &&
                    full_len > $buffer_capacity
                        .try_into()
                        .map_err(|e| common::Error::wrapper(format!("{:?}", e)))?) {

                assert!(full_len > 0);
                let full_len: usize = full_len
                    .try_into()
                    .map_err(|e| common::Error::wrapper(format!("{:?}", e)))?;
                buf.resize(full_len, 0);

                // Same unsafe requirements as above, plus full_len must be exactly the output
                // buffer size.
                unsafe {
                    assert!(common::Error::is_ok(status));
                    uloc_method(
                        $($before_arg,)*
                        buf.as_mut_ptr() as *mut raw::c_char,
                        full_len as i32,
                        $($after_arg,)*
                        &mut status,
                    )
                };
            }

            common::Error::ok_or_warning(status)?;

            // Adjust the size of the buffer here.
            if (full_len > 0) {
                let full_len: usize = full_len
                    .try_into()
                    .map_err(|e| common::Error::wrapper(format!("{:?}", e)))?;
                buf.resize(full_len, 0);
            }
            String::from_utf8(buf).map_err(|_| common::Error::string_with_interior_nul())
        }
    }
}

impl ULoc {
    /// Implements `uloc_getLanguage`.
    pub fn language(&self) -> Result<String, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_getLanguage))
    }

    /// Implements `uloc_getScript`.
    pub fn script(&self) -> Result<String, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_getScript))
    }

    /// Implements `uloc_getCountry`.
    pub fn country(&self) -> Result<String, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_getCountry))
    }

    /// Implements `uloc_getVariant`.
    pub fn variant(&self) -> Result<String, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_getVariant))
    }

    /// Implements `uloc_canonicalize` from ICU4C.
    pub fn canonicalize(&self) -> Result<ULoc, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_canonicalize))
            .map(|repr| ULoc { repr })
    }

    /// Implements `uloc_addLikelySubtags` from ICU4C.
    pub fn add_likely_subtags(&self) -> Result<ULoc, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_addLikelySubtags))
            .map(|repr| ULoc { repr })
    }

    /// Implements `uloc_minimizeSubtags` from ICU4C.
    pub fn minimize_subtags(&self) -> Result<ULoc, common::Error> {
        self.call_buffered_string_method(versioned_function!(uloc_minimizeSubtags))
            .map(|repr| ULoc { repr })
    }

    // Implements `uloc_toLanguageTag` from ICU4C.
    pub fn to_language_tag(&self, strict: bool) -> Result<String, common::Error> {
        buffered_string_method_with_retry!(
            buffered_string_to_language_tag,
            LOCALE_CAPACITY,
            [locale_id: *const raw::c_char,],
            [strict: rust_icu_sys::UBool,]
        );

        let locale_id = self.as_c_str();
        // No `UBool` constants available in rust_icu_sys, unfortunately.
        let strict = if strict { 1 } else { 0 };
        buffered_string_to_language_tag(
            versioned_function!(uloc_toLanguageTag),
            locale_id.as_ptr(),
            strict,
        )
    }

    /// Returns the current label of this locale.
    pub fn label(&self) -> &str {
        &self.repr
    }

    /// Returns the current locale name as a C string.
    pub fn as_c_str(&self) -> ffi::CString {
        ffi::CString::new(self.repr.clone()).expect("ULoc contained interior NUL bytes")
    }

    // Implements `uloc_acceptLanguage` from ICU4C.
    #[deprecated = "Use `rust_icu_uloc::accept_language`"]
    pub fn accept_language(
        accept_list: impl IntoIterator<Item = impl Into<ULoc>>,
        available_locales: impl IntoIterator<Item = impl Into<ULoc>>,
    ) -> Result<(Option<ULoc>, UAcceptResult), common::Error> {
        accept_language(accept_list, available_locales)
    }

    /// Call a `uloc` method that takes this locale's ID and returns a string.
    fn call_buffered_string_method(
        &self,
        uloc_method: unsafe extern "C" fn(
            *const raw::c_char,
            *mut raw::c_char,
            i32,
            *mut UErrorCode,
        ) -> i32,
    ) -> Result<String, common::Error> {
        buffered_string_method_with_retry!(
            buffered_string_char_star,
            LOCALE_CAPACITY,
            [char_star: *const raw::c_char,],
            []
        );
        let asciiz = self.as_c_str();
        buffered_string_char_star(uloc_method, asciiz.as_ptr())
    }
}

/// Gets the current system default locale.
///
/// Implements `uloc_getDefault` from ICU4C.
pub fn get_default() -> ULoc {
    let loc = unsafe { versioned_function!(uloc_getDefault)() };
    let uloc_cstr = unsafe { ffi::CStr::from_ptr(loc) };
    crate::ULoc::try_from(uloc_cstr).expect("could not convert default locale to ULoc")
}

/// Sets the current default system locale.
///
/// Implements `uloc_setDefault` from ICU4C.
pub fn set_default(loc: &ULoc) -> Result<(), common::Error> {
    let mut status = common::Error::OK_CODE;
    let asciiz = ffi::CString::new(loc.repr.clone())
        .map_err(|_| common::Error::string_with_interior_nul())?;
    unsafe { versioned_function!(uloc_setDefault)(asciiz.as_ptr(), &mut status) };
    common::Error::ok_or_warning(status)
}

// Implements `uloc_acceptLanguage` from ICU4C.
pub fn accept_language(
    accept_list: impl IntoIterator<Item = impl Into<ULoc>>,
    available_locales: impl IntoIterator<Item = impl Into<ULoc>>,
) -> Result<(Option<ULoc>, UAcceptResult), common::Error> {
    buffered_string_method_with_retry!(
        buffered_string_uloc_accept_language,
        LOCALE_CAPACITY,
        [],
        [
            out_result: *mut UAcceptResult,
            accept_list: *mut *const ::std::os::raw::c_char,
            accept_list_count: i32,
            available_locales: *mut UEnumeration,
        ]
    );

    let mut accept_result: UAcceptResult = UAcceptResult::ULOC_ACCEPT_FAILED;
    let mut accept_list_cstrings: Vec<ffi::CString> = vec![];
    // This is mutable only to satisfy the missing `const`s in the ICU4C API.
    let mut accept_list: Vec<*const raw::c_char> = accept_list
        .into_iter()
        .map(|item| {
            let uloc: ULoc = item.into();
            accept_list_cstrings.push(uloc.as_c_str());
            accept_list_cstrings
                .last()
                .expect("non-empty list")
                .as_ptr()
        })
        .collect();

    let available_locales: Vec<ULoc> = available_locales
        .into_iter()
        .map(|item| item.into())
        .collect();
    let available_locales: Vec<&str> = available_locales.iter().map(|uloc| uloc.label()).collect();
    let mut available_locales = Enumeration::try_from(&available_locales[..])?;

    let matched_locale = buffered_string_uloc_accept_language(
        versioned_function!(uloc_acceptLanguage),
        &mut accept_result,
        accept_list.as_mut_ptr(),
        accept_list.len() as i32,
        available_locales.repr(),
    );

    // Having no match is a valid if disappointing result.
    if accept_result == UAcceptResult::ULOC_ACCEPT_FAILED {
        return Ok((None, accept_result));
    }

    matched_locale
        .and_then(|s| ULoc::try_from(s.as_str()))
        .map(|uloc| (Some(uloc), accept_result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language() {
        let loc = ULoc::try_from("es-CO").expect("get es_CO locale");
        let language = loc.language().expect("should get language");
        assert_eq!(&language, "es");
    }

    #[test]
    fn test_script() {
        let loc = ULoc::try_from("sr-Cyrl").expect("get sr_Cyrl locale");
        let script = loc.script().expect("should get script");
        assert_eq!(&script, "Cyrl");
    }

    #[test]
    fn test_country() {
        let loc = ULoc::try_from("es-CO").expect("get es_CO locale");
        let country = loc.country().expect("should get country");
        assert_eq!(&country, "CO");
    }

    #[test]
    fn test_variant() {
        let loc = ULoc::try_from("zh-Latn-pinyin").expect("get zh_Latn_pinyin locale");
        let variant = loc.variant().expect("should get variant");
        assert_eq!(&variant, "PINYIN");
    }

    #[test]
    fn test_default_locale() {
        let loc = ULoc::try_from("fr-fr").expect("get fr_FR locale");
        set_default(&loc).expect("successful set of locale");
        assert_eq!(get_default().label(), loc.label());
        assert_eq!(loc.label(), "fr_FR", "The locale should get canonicalized");
        let loc = ULoc::try_from("en-us").expect("get en_US locale");
        set_default(&loc).expect("successful set of locale");
        assert_eq!(get_default().label(), loc.label());
    }

    #[test]
    fn test_add_likely_subtags() {
        let loc = ULoc::try_from("en-US").expect("get en_US locale");
        let with_likely_subtags = loc.add_likely_subtags().expect("should add likely subtags");
        let expected = ULoc::try_from("en_Latn_US").expect("get en_Latn_US locale");
        assert_eq!(with_likely_subtags.label(), expected.label());
    }

    #[test]
    fn test_minimize_subtags() {
        let loc = ULoc::try_from("sr_Cyrl_RS").expect("get sr_Cyrl_RS locale");
        let minimized_subtags = loc.minimize_subtags().expect("should minimize subtags");
        let expected = ULoc::try_from("sr").expect("get sr locale");
        assert_eq!(minimized_subtags.label(), expected.label());
    }

    #[test]
    fn test_to_language_tag() {
        let loc = ULoc::try_from("sr_Cyrl_RS").expect("get sr_Cyrl_RS locale");
        let language_tag = loc
            .to_language_tag(true)
            .expect("should convert to language tag");
        assert_eq!(language_tag, "sr-Cyrl-RS".to_string());
    }

    #[test]
    fn test_accept_language_fallback() {
        let accept_list: Result<Vec<_>, _> = vec!["es_MX", "ar_EG", "fr_FR"]
            .into_iter()
            .map(ULoc::try_from)
            .collect();
        let accept_list = accept_list.expect("make accept_list");

        let available_locales: Result<Vec<_>, _> =
            vec!["de_DE", "en_US", "es", "nl_NL", "sr_RS_Cyrl"]
                .into_iter()
                .map(ULoc::try_from)
                .collect();
        let available_locales = available_locales.expect("make available_locales");

        let actual = accept_language(accept_list, available_locales).expect("call accept_language");
        assert_eq!(
            actual,
            (
                ULoc::try_from("es").ok(),
                UAcceptResult::ULOC_ACCEPT_FALLBACK
            )
        );
    }

    #[test]
    fn test_accept_language_exact_match() {
        let accept_list: Result<Vec<_>, _> = vec!["es_ES", "ar_EG", "fr_FR"]
            .into_iter()
            .map(ULoc::try_from)
            .collect();
        let accept_list = accept_list.expect("make accept_list");

        let available_locales: Result<Vec<_>, _> = vec!["de_DE", "en_US", "es_MX", "ar_EG"]
            .into_iter()
            .map(ULoc::try_from)
            .collect();
        let available_locales = available_locales.expect("make available_locales");

        let actual = accept_language(accept_list, available_locales).expect("call accept_language");
        assert_eq!(
            actual,
            (
                ULoc::try_from("ar_EG").ok(),
                UAcceptResult::ULOC_ACCEPT_VALID
            )
        );
    }

    #[test]
    fn test_accept_language_no_match() {
        let accept_list: Result<Vec<_>, _> = vec!["es_ES", "ar_EG", "fr_FR"]
            .into_iter()
            .map(ULoc::try_from)
            .collect();
        let accept_list = accept_list.expect("make accept_list");

        let available_locales: Result<Vec<_>, _> =
            vec!["el_GR"].into_iter().map(ULoc::try_from).collect();
        let available_locales = available_locales.expect("make available_locales");

        let actual = accept_language(accept_list, available_locales).expect("call accept_language");
        assert_eq!(actual, (None, UAcceptResult::ULOC_ACCEPT_FAILED))
    }
}
