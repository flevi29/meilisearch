mod adjust_indexes;
mod best_match_interval;

use std::cmp::{max, min};

use super::r#match::{Match, MatchPosition};

use adjust_indexes::{
    get_adjusted_index_forward_for_crop_size, get_adjusted_indexes_for_highlights_and_crop_size,
};
use charabia::Token;
use serde::Serialize;

use super::FormatOptions;

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MatchBounds {
    Full,
    Formatted { highlight_toggle: bool, indexes: Box<[usize]> },
}

pub struct MatchBoundsHelper<'a> {
    tokens: &'a [Token<'a>],
    matches: &'a [Match],
}

impl MatchBoundsHelper<'_> {
    fn get_match_byte_position_range(&self, index: usize) -> [usize; 2] {
        let r#match = &self.matches[index];

        let byte_start = match r#match.position {
            MatchPosition::Word { token_position, .. } => self.tokens[token_position].byte_start,
            MatchPosition::Phrase { token_position_range: [ftp, ..], .. } => {
                self.tokens[ftp].byte_start
            }
        };

        [byte_start, byte_start + r#match.byte_len - 1]
    }

    /// TODO: Description
    fn get_match_bounds(
        &self,
        matches_first_index: usize,
        matches_last_index: usize,
        crop_byte_start: usize,
        crop_byte_end: usize,
    ) -> MatchBounds {
        let [first_match_first_byte, first_match_last_byte] =
            self.get_match_byte_position_range(matches_first_index);
        let first_match_first_byte = max(first_match_first_byte, crop_byte_start);

        let [last_match_first_byte, last_match_last_byte] =
            if matches_first_index != matches_last_index {
                self.get_match_byte_position_range(matches_last_index)
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

        // TODO: This should consider matches that are not in the index range as well

        indexes.push(first_match_first_byte);

        if selected_matches_len > 1 {
            indexes.push(first_match_last_byte);
        }

        if selected_matches_len > 2 {
            let mut index = matches_first_index + 1;
            while index != matches_last_index {
                let [m_byte_start, m_byte_end] = self.get_match_byte_position_range(index);

                indexes.push(m_byte_start);
                indexes.push(m_byte_end);

                index += 1;
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
            indexes: indexes.into(),
        }
    }

    fn get_crop_bounds(&self, crop_size: usize) -> MatchBounds {
        let final_token_index = get_adjusted_index_forward_for_crop_size(self.tokens, crop_size);
        let final_token = &self.tokens[final_token_index];
        let crop_byte_end = if final_token_index != self.tokens.len() - 1 {
            final_token.byte_start
        } else {
            final_token.byte_end
        };

        MatchBounds::Formatted { highlight_toggle: false, indexes: Box::new([0, crop_byte_end]) }
    }

    fn get_crop_and_highlight_bounds_thingy(&self, crop_size: usize) -> [usize; 4] {
        let [matches_first_index, matches_last_index] =
            best_match_interval::get_best_match_interval(self.matches, crop_size);

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

        [matches_first_index, matches_last_index, crop_byte_start, crop_byte_end]
    }

    /// TODO: description
    fn get_crop_and_highlight_bounds(&self, crop_size: usize) -> MatchBounds {
        let [matches_first_index, matches_last_index, crop_byte_start, crop_byte_end] =
            self.get_crop_and_highlight_bounds_thingy(crop_size);

        self.get_match_bounds(
            matches_first_index,
            matches_last_index,
            crop_byte_start,
            crop_byte_end,
        )
    }

    // TODO: Rename
    fn asd(&self, crop_size: usize) -> MatchBounds {
        let [_, _, crop_byte_start, crop_byte_end] =
            self.get_crop_and_highlight_bounds_thingy(crop_size);

        MatchBounds::Formatted {
            highlight_toggle: false,
            indexes: Box::new([crop_byte_start, crop_byte_end]),
        }
    }
}

pub fn get_match_bounds(
    tokens: &[Token],
    matches: &[Match],
    format_options: FormatOptions,
) -> MatchBounds {
    let mbh = MatchBoundsHelper { tokens, matches };

    if let Some(crop_size) = format_options.crop.filter(|v| *v != 0) {
        if matches.is_empty() {
            return mbh.get_crop_bounds(crop_size);
        }

        if format_options.highlight {
            return mbh.get_crop_and_highlight_bounds(crop_size);
        }

        return mbh.asd(crop_size);
    }

    if format_options.highlight && !matches.is_empty() {
        mbh.get_match_bounds(0, matches.len() - 1, 0, tokens[tokens.len() - 1].byte_end)
    } else {
        MatchBounds::Full
    }
}
