mod adjust_indexes;
mod best_match_range;

use std::{
    borrow::Cow,
    cmp::{max, min},
};

use super::{
    matching_words::QueryPosition,
    r#match::{Match, MatchPosition},
    MarkerOptions,
};

use adjust_indexes::{
    get_adjusted_index_forward_for_crop_size, get_adjusted_indexes_for_highlights_and_crop_size,
};
use charabia::Token;
use serde::Serialize;

use super::FormatOptions;

// TODO: https://github.com/meilisearch/meilisearch/pull/5005/files
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MatchBounds {
    Full,
    Formatted { highlight_toggle: bool, indexes: Vec<usize> },
}

struct MatchBoundsHelper<'a> {
    tokens: &'a [Token<'a>],
    matches: &'a [Match],
    query_positions: &'a [QueryPosition],
}

struct MatchesAndCropIndexes {
    matches_first_index: usize,
    matches_last_index: usize,
    crop_byte_start: usize,
    crop_byte_end: usize,
}

enum CropThing {
    Last(usize),
    First(usize),
}

impl MatchBoundsHelper<'_> {
    fn get_match_byte_position_range(&self, r#match: &Match) -> [usize; 2] {
        let byte_start = match r#match.position {
            MatchPosition::Word { token_position, .. } => self.tokens[token_position].byte_start,
            MatchPosition::Phrase { token_position_range: [ftp, ..], .. } => {
                self.tokens[ftp].byte_start
            }
        };

        [byte_start, byte_start + r#match.byte_len]
    }

    fn get_match_byte_position_rangee(
        &self,
        index: &mut usize,
        crop_thing: CropThing,
    ) -> [usize; 2] {
        let new_index = match crop_thing {
            CropThing::First(_) if *index != 0 => *index - 1,
            CropThing::Last(_) if *index != self.matches.len() - 1 => *index + 1,
            _ => {
                return self.get_match_byte_position_range(&self.matches[*index]);
            }
        };

        let [byte_start, byte_end] = self.get_match_byte_position_range(&self.matches[new_index]);

        // NOTE: This doesn't need additional checks, because `get_best_match_index_range` already
        // guarantees that the next or preceeding match contains the crop boundary
        match crop_thing {
            CropThing::First(crop_byte_start) if crop_byte_start < byte_end => {
                *index -= 1;
                [byte_start, byte_end]
            }
            CropThing::Last(crop_byte_end) if byte_start < crop_byte_end => {
                *index += 1;
                [byte_start, byte_end]
            }
            _ => self.get_match_byte_position_range(&self.matches[*index]),
        }
    }

    /// TODO: Description
    fn get_match_bounds(&self, mci: MatchesAndCropIndexes) -> MatchBounds {
        let MatchesAndCropIndexes {
            mut matches_first_index,
            mut matches_last_index,
            crop_byte_start,
            crop_byte_end,
        } = mci;

        let [first_match_first_byte, first_match_last_byte] = self.get_match_byte_position_rangee(
            &mut matches_first_index,
            CropThing::First(crop_byte_start),
        );
        let first_match_first_byte = max(first_match_first_byte, crop_byte_start);

        let [last_match_first_byte, last_match_last_byte] =
            if matches_first_index != matches_last_index {
                self.get_match_byte_position_rangee(
                    &mut matches_last_index,
                    CropThing::Last(crop_byte_end),
                )
            } else {
                [first_match_first_byte, first_match_last_byte]
            };
        let last_match_last_byte = min(last_match_last_byte, crop_byte_end);

        let selected_matches_len = matches_last_index - matches_first_index + 1;
        let mut indexes_size = 2 * selected_matches_len;

        let crop_byte_start_is_not_first_match_start = crop_byte_start != first_match_first_byte;
        let crop_byte_end_is_not_last_match_end = crop_byte_end != last_match_last_byte;

        if crop_byte_start_is_not_first_match_start {
            indexes_size += 1;
        }

        if crop_byte_end_is_not_last_match_end {
            indexes_size += 1;
        }

        let mut indexes = Vec::with_capacity(indexes_size);

        if crop_byte_start_is_not_first_match_start {
            indexes.push(crop_byte_start);
        }

        indexes.push(first_match_first_byte);

        if selected_matches_len > 1 {
            indexes.push(first_match_last_byte);
        }

        if selected_matches_len > 2 {
            for index in (matches_first_index + 1)..matches_last_index {
                let [m_byte_start, m_byte_end] =
                    self.get_match_byte_position_range(&self.matches[index]);

                indexes.push(m_byte_start);
                indexes.push(m_byte_end);
            }
        }

        if selected_matches_len > 1 {
            indexes.push(last_match_first_byte);
        }

        indexes.push(last_match_last_byte);

        if crop_byte_end_is_not_last_match_end {
            indexes.push(crop_byte_end);
        }

        MatchBounds::Formatted {
            highlight_toggle: !crop_byte_start_is_not_first_match_start,
            indexes,
        }
    }

    /// For crop but no highlight.
    fn get_crop_bounds_with_no_matches(&self, crop_size: usize) -> MatchBounds {
        let final_token_index = get_adjusted_index_forward_for_crop_size(self.tokens, crop_size);
        let final_token = &self.tokens[final_token_index];
        let crop_byte_end = if final_token_index != self.tokens.len() - 1 {
            final_token.byte_start
        } else {
            final_token.byte_end
        };

        MatchBounds::Formatted { highlight_toggle: false, indexes: vec![0, crop_byte_end] }
    }

    fn get_matches_and_crop_indexes(&self, crop_size: usize) -> MatchesAndCropIndexes {
        // TODO: This doesnt give back 2 phrases if one is out of crop window
        // Solution: also get next and previous matches, and if they're in the crop window, even if partially, highlight them
        let [matches_first_index, matches_last_index] =
            best_match_range::get_best_match_index_range(
                self.matches,
                self.query_positions,
                crop_size,
            );

        let first_match = &self.matches[matches_first_index];
        let last_match = &self.matches[matches_last_index];

        let words_count = last_match.get_last_word_pos() - first_match.get_first_word_pos() + 1;
        let [index_backward, index_forward] = get_adjusted_indexes_for_highlights_and_crop_size(
            self.tokens,
            first_match.get_first_token_pos(),
            last_match.get_last_token_pos(),
            words_count,
            crop_size,
        );

        let is_index_backward_at_limit = index_backward == 0;
        let is_index_forward_at_limit = index_forward == self.tokens.len() - 1;

        let backward_token = &self.tokens[index_backward];
        let crop_byte_start = if is_index_backward_at_limit {
            backward_token.byte_start
        } else {
            backward_token.byte_end
        };

        let forward_token = &self.tokens[index_forward];
        let crop_byte_end = if is_index_forward_at_limit {
            forward_token.byte_end
        } else {
            forward_token.byte_start
        };

        MatchesAndCropIndexes {
            matches_first_index,
            matches_last_index,
            crop_byte_start,
            crop_byte_end,
        }
    }

    /// For when
    fn get_crop_and_highlight_bounds(&self, crop_size: usize) -> MatchBounds {
        self.get_match_bounds(self.get_matches_and_crop_indexes(crop_size))
    }

    /// For when there are no matches, but crop is required.
    fn get_crop_bounds_with_matches(&self, crop_size: usize) -> MatchBounds {
        let mci = self.get_matches_and_crop_indexes(crop_size);

        MatchBounds::Formatted {
            highlight_toggle: false,
            indexes: vec![mci.crop_byte_start, mci.crop_byte_end],
        }
    }
}

impl MatchBounds {
    pub fn new(
        tokens: &[Token],
        matches: &[Match],
        query_positions: &[QueryPosition],
        array_indices: &[usize],
        format_options: FormatOptions,
    ) -> Self {
        let mbh = MatchBoundsHelper { tokens, matches, query_positions };

        if let Some(crop_size) = format_options.crop.filter(|v| *v != 0) {
            if matches.is_empty() {
                return mbh.get_crop_bounds_with_no_matches(crop_size);
            }

            if format_options.highlight {
                return mbh.get_crop_and_highlight_bounds(crop_size);
            }

            return mbh.get_crop_bounds_with_matches(crop_size);
        }

        if format_options.highlight && !matches.is_empty() {
            mbh.get_match_bounds(MatchesAndCropIndexes {
                matches_first_index: 0,
                matches_last_index: matches.len() - 1,
                crop_byte_start: 0,
                crop_byte_end: tokens[tokens.len() - 1].byte_end,
            })
        } else {
            Self::Full
        }
    }

    pub fn to_formatted_text<'a>(&self, text: &'a str, options: &MarkerOptions) -> Cow<'a, str> {
        let Self::Formatted { mut highlight_toggle, indexes } = self else {
            return Cow::Borrowed(text);
        };

        let mut formatted_text = Vec::new();

        let mut indexes_iter = indexes.iter();
        let mut previous_index = indexes_iter.next().expect("TODO");

        // push crop marker if it's not the start of the text
        if !options.crop_marker.is_empty() && *previous_index != 0 {
            formatted_text.push(options.crop_marker.as_str());
        }

        for index in indexes_iter {
            if highlight_toggle {
                formatted_text.push(options.highlight_pre_tag.as_str());
            }

            formatted_text.push(&text[*previous_index..*index]);

            if highlight_toggle {
                formatted_text.push(options.highlight_post_tag.as_str());
            }

            highlight_toggle = !highlight_toggle;
            previous_index = index;
        }

        // push crop marker if it's not the end of the text
        if !options.crop_marker.is_empty() && *previous_index < text.len() {
            formatted_text.push(options.crop_marker.as_str());
        }

        if formatted_text.len() == 1 {
            // avoid concatenating if there is only one element
            return Cow::Owned(formatted_text[0].to_string());
        }

        Cow::Owned(formatted_text.concat())
    }
}
