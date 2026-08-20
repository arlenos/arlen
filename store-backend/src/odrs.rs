// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The ODRS read-only client: what other people made of an app, as one row.
//!
//! `store-app.md` section 2 decided this on 23 July - read-only from day one, one
//! signal beside the capability manifest, **never the headline and never a star
//! ranking**. Writing reviews and running a moderated service stay out of scope.
//! If Arlen ever self-hosts ODRS the same client points at that instance, so
//! nothing here hardcodes the host beyond a default.
//!
//! This module is the PURE half: parse the ratings document, compute a score,
//! answer for an id. Fetching it is separate on purpose - `main.rs` says the
//! backend performs no network I/O and its unit denies egress outright, so the
//! first request out of this daemon is a posture change rather than a detail,
//! and it lands with the allowlist that permits it.

use std::collections::BTreeMap;

/// One app's rating tally, as ODRS publishes it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Tally {
    /// Reviews that carry no star rating at all.
    ///
    /// NOT a zero-star review, and the difference is the whole trap in this
    /// file: ODRS counts "somebody wrote something and rated nothing" here.
    /// Averaging it in as a zero drags every score toward the floor - an app
    /// with five five-star reviews and five unrated ones would read 2.5.
    #[serde(default)]
    pub star0: u32,
    #[serde(default)]
    pub star1: u32,
    #[serde(default)]
    pub star2: u32,
    #[serde(default)]
    pub star3: u32,
    #[serde(default)]
    pub star4: u32,
    #[serde(default)]
    pub star5: u32,
    /// What ODRS reports as the total, which INCLUDES `star0`.
    #[serde(default)]
    pub total: u32,
}

impl Tally {
    /// How many of these reviews actually carry a rating.
    #[must_use]
    pub fn rated(&self) -> u32 {
        self.star1 + self.star2 + self.star3 + self.star4 + self.star5
    }

    /// The mean rating, or `None` when nobody rated it.
    ///
    /// `None` rather than 0.0: an app nobody has rated and an app everybody
    /// hated are different facts, and a surface that renders both as an empty
    /// row is at least not lying about the second.
    #[must_use]
    pub fn score(&self) -> Option<f32> {
        let rated = self.rated();
        if rated == 0 {
            return None;
        }
        let sum = self.star1 + 2 * self.star2 + 3 * self.star3 + 4 * self.star4 + 5 * self.star5;
        Some(f64::from(sum) as f32 / rated as f32)
    }
}

/// Every app ODRS knows about, keyed the way ODRS keys them.
#[derive(Debug, Clone, Default)]
pub struct Ratings(BTreeMap<String, Tally>);

impl Ratings {
    /// Parse the `/1.0/reviews/api/ratings` document.
    ///
    /// # Errors
    /// When the document is not the object-of-tallies shape.
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str::<BTreeMap<String, Tally>>(json)
            .map(Ratings)
            .map_err(|e| format!("odrs ratings: {e}"))
    }

    /// The score for a component id.
    ///
    /// ODRS keys most entries by the DESKTOP id (`0ad.desktop`) while the store
    /// carries AppStream component ids, which for a Flatpak is usually the
    /// reverse-DNS form without the suffix. Both spellings are tried rather than
    /// picking one and quietly answering `None` for the other half of the
    /// catalogue.
    #[must_use]
    pub fn score_for(&self, id: &str) -> Option<f32> {
        self.0
            .get(id)
            .or_else(|| self.0.get(&format!("{id}.desktop")))
            .or_else(|| self.0.get(id.strip_suffix(".desktop").unwrap_or(id)))
            .and_then(Tally::score)
    }

    /// How many apps this document covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether it covers nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{
      "0ad.desktop":  {"star0": 0, "star1": 3, "star2": 1, "star3": 0, "star4": 7, "star5": 18, "total": 29},
      "org.gnome.gitg": {"star0": 4, "star1": 0, "star2": 0, "star3": 0, "star4": 0, "star5": 4, "total": 8},
      "nobody.desktop": {"star0": 6, "star1": 0, "star2": 0, "star3": 0, "star4": 0, "star5": 0, "total": 6}
    }"#;

    #[test]
    fn a_score_is_the_mean_of_the_ratings_people_gave() {
        let r = Ratings::parse(DOC).unwrap();
        // (3*1 + 1*2 + 7*4 + 18*5) / 29 = 123/29
        let s = r.score_for("0ad.desktop").unwrap();
        assert!((s - 123.0 / 29.0).abs() < 0.001, "got {s}");
    }

    #[test]
    fn unrated_reviews_do_not_drag_the_score_down() {
        // THE trap. `star0` is "wrote something, rated nothing", and `total`
        // counts it - so dividing the star sum by `total` would report 2.5 for
        // an app whose every rating is five stars.
        let r = Ratings::parse(DOC).unwrap();
        let s = r.score_for("org.gnome.gitg").unwrap();
        assert!((s - 5.0).abs() < 0.001, "four five-star ratings is 5.0, got {s}");
    }

    #[test]
    fn an_app_nobody_rated_has_no_score_rather_than_a_zero() {
        let r = Ratings::parse(DOC).unwrap();
        assert_eq!(r.score_for("nobody.desktop"), None);
    }

    #[test]
    fn an_id_is_found_with_or_without_the_desktop_suffix() {
        let r = Ratings::parse(DOC).unwrap();
        // The store carries AppStream ids; ODRS keys most entries by desktop id.
        assert!(r.score_for("0ad").is_some(), "bare id finds the .desktop key");
        assert!(r.score_for("org.gnome.gitg.desktop").is_some(), "and the other way round");
    }

    #[test]
    fn an_app_odrs_never_heard_of_has_no_score() {
        let r = Ratings::parse(DOC).unwrap();
        assert_eq!(r.score_for("dev.arlen.mail"), None);
    }

    #[test]
    fn a_document_that_is_not_ratings_is_refused_rather_than_read_as_empty() {
        // An empty catalogue and a document we could not understand are
        // different, and only one of them means "nobody has rated anything".
        assert!(Ratings::parse("[]").is_err());
        assert!(Ratings::parse("not json").is_err());
    }
}
