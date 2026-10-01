//! Query-string extractor whose rejections are validation errors.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::Error;

/// Like axum's `Query`, but a query string that does not deserialize (a
/// repeated key such as `?tags=a&tags=b`, or `page=abc`) is rejected with
/// [`Error::Validation`], so the API answers with its JSON error body and the
/// website can show the message inline, instead of axum's plain-text
/// rejection. The message names the offending parameter.
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidatedQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parse_query(parts.uri.query().unwrap_or_default()).map(ValidatedQuery)
    }
}

/// Deserialize a raw query string, naming the parameter in the error.
pub fn parse_query<T: DeserializeOwned>(query: &str) -> Result<T, Error> {
    let deserializer =
        serde_urlencoded::Deserializer::new(url::form_urlencoded::parse(query.as_bytes()));
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let parameter = error.path().to_string();
        let message = error.into_inner().to_string();
        Error::Validation(if message.starts_with("duplicate field") {
            format!(
                "Invalid query string: {message}; give each parameter once \
                 (list filters are comma-separated)"
            )
        } else if parameter.is_empty() || parameter == "." {
            format!("Invalid query string: {message}")
        } else {
            format!("Invalid value for {parameter}: {message}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::SearchParams;

    fn message(query: &str) -> String {
        match parse_query::<SearchParams>(query) {
            Err(Error::Validation(message)) => message,
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_number_names_its_parameter() {
        assert_eq!(
            message("page=abc"),
            "Invalid value for page: invalid digit found in string"
        );
        assert!(message("q=x&limit=abc").starts_with("Invalid value for limit:"));
    }

    #[test]
    fn a_repeated_parameter_says_to_give_it_once() {
        let message = message("tags=a&tags=b");
        assert!(message.contains("duplicate field `tags`"), "{message}");
        assert!(message.contains("comma-separated"), "{message}");
    }

    #[test]
    fn a_valid_query_string_deserializes() {
        let params = parse_query::<SearchParams>("q=soup&page=2&tags=a,b").unwrap();
        assert_eq!(params.q, "soup");
        assert_eq!(params.page, 2);
        assert_eq!(params.filters.tags.as_deref(), Some("a,b"));
    }
}
