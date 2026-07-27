use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizationCatalog {
    pub fallback_locale: String,
    #[serde(default)]
    pub locales: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LocalizationError {
    #[error("missing localization key '{key}' for locale '{locale}'")]
    MissingKey { locale: String, key: String },
    #[error("localized text references missing token '{0}'")]
    MissingToken(String),
    #[error("localized text contains an unterminated token")]
    UnterminatedToken,
}

impl LocalizationCatalog {
    pub fn resolve(
        &self,
        locale: &str,
        key: &str,
        tokens: &BTreeMap<String, String>,
    ) -> Result<String, LocalizationError> {
        let template = self
            .locales
            .get(locale)
            .and_then(|entries| entries.get(key))
            .or_else(|| {
                self.locales
                    .get(&self.fallback_locale)
                    .and_then(|entries| entries.get(key))
            })
            .ok_or_else(|| LocalizationError::MissingKey {
                locale: locale.into(),
                key: key.into(),
            })?;
        interpolate(template, tokens)
    }
}

fn interpolate(
    template: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<String, LocalizationError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        output.push_str(&remaining[..start]);
        remaining = &remaining[start + 1..];
        let end = remaining
            .find('}')
            .ok_or(LocalizationError::UnterminatedToken)?;
        let key = &remaining[..end];
        let value = tokens
            .get(key)
            .ok_or_else(|| LocalizationError::MissingToken(key.into()))?;
        output.push_str(value);
        remaining = &remaining[end + 1..];
    }
    output.push_str(remaining);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_falls_back_and_interpolates() {
        let catalog = LocalizationCatalog {
            fallback_locale: "en".into(),
            locales: BTreeMap::from([(
                "en".into(),
                BTreeMap::from([("welcome".into(), "Welcome, {name}!".into())]),
            )]),
        };
        assert_eq!(
            catalog
                .resolve(
                    "zh-CN",
                    "welcome",
                    &BTreeMap::from([("name".into(), "Cloud".into())])
                )
                .unwrap(),
            "Welcome, Cloud!"
        );
    }
}
