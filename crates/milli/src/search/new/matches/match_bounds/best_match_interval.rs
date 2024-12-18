use std::cell::Cell;

use super::super::matching_words::UserQueryPositionRange;
use super::super::Match;

struct MatchesIndexRangeWithScore {
    matches_index_range: [usize; 2],
    score: [i16; 3],
}

/// Compute the score of a match interval:
/// 1) count unique matches
/// 2) calculate distance between matches
/// 3) count ordered matches
fn get_score(matches: &[Match]) -> [i16; 3] {
    let mut uniqueness_score = 0i16;
    let mut current_range: Option<UserQueryPositionRange> = None;
    // matches are always ordered, so +1 for each match
    let order_score = Cell::new(matches.len() as i16);
    let distance_score = Cell::new(0);

    // count score for phrases
    let tally_phrase_scores = |fwp, lwp| {
        let words_in_phrase_minus_one = (lwp - fwp) as i16;
        // will always be ordered, so +1 for each space between words
        order_score.set(order_score.get() + words_in_phrase_minus_one);
        // distance will always be 1, so -1 for each space between words
        distance_score.set(distance_score.get() - words_in_phrase_minus_one);
    };

    let mut iter = matches.iter().peekable();
    while let Some(r#match) = iter.next() {
        if let Some(next_match) = iter.peek() {
            let match_last_word_pos = match *r#match {
                Match::Word { word_position, .. } => word_position,
                Match::Phrase { word_position_range: [fwp, lwp], .. } => {
                    tally_phrase_scores(fwp, lwp);
                    lwp
                }
            };
            let next_match_first_word_pos = next_match.get_first_word_pos();

            // compute distance between matches
            distance_score.set(
                distance_score.get()
                    - (next_match_first_word_pos - match_last_word_pos).min(7) as i16,
            );
        } else if let Match::Phrase { word_position_range: [fwp, lwp], .. } = *r#match {
            // in case last match is a phrase, count score for its words
            tally_phrase_scores(fwp, lwp);
        }

        // because matches are ordered by query position, this algorithm avoids needing a vector
        let query_position = r#match.get_query_position();
        match current_range.as_mut() {
            Some([saved_range_start, saved_range_end]) => {
                let [range_start, range_end] = query_position;

                if range_start > *saved_range_start {
                    uniqueness_score += (*saved_range_end - *saved_range_start) as i16 + 1;

                    *saved_range_start = range_start;
                    *saved_range_end = range_end;
                } else if range_end > *saved_range_end {
                    *saved_range_end = range_end;
                }
            }
            None => current_range = Some(query_position),
        }
    }

    if let Some([saved_range_start, saved_range_end]) = current_range {
        uniqueness_score += (saved_range_end - saved_range_start) as i16 + 1;
    }

    // rank by unique match count, then by distance between matches, then by ordered match count.
    [uniqueness_score, distance_score.into_inner(), order_score.into_inner()]
}

/// Returns the first and last match where the score computed by match_interval_score is the best.
pub fn get_best_match_interval(matches: &[Match], crop_size: usize) -> [usize; 2] {
    // positions of the first and the last match of the best matches interval in `matches`.
    let mut best_matches_index_range: Option<MatchesIndexRangeWithScore> = None;

    let mut save_best_interval = |interval_first, interval_last| {
        let score = get_score(&matches[interval_first..=interval_last]);
        let is_score_better = best_matches_index_range.as_ref().map_or(true, |v| score > v.score);

        if is_score_better {
            best_matches_index_range = Some(MatchesIndexRangeWithScore {
                matches_index_range: [interval_first, interval_last],
                score,
            });
        }
    };

    // we compute the matches interval if we have at least 2 matches.
    // current interval positions.
    let mut interval_first = 0;
    let mut interval_first_match_first_word_pos = matches[interval_first].get_first_word_pos();

    for (index, next_match) in matches.iter().enumerate() {
        // if next match would make interval gross more than crop_size,
        // we compare the current interval with the best one,
        // then we increase `interval_first` until next match can be added.
        let next_match_last_word_pos = next_match.get_last_word_pos();

        // if the next match would mean that we pass the crop size window,
        // we take the last valid match, that didn't pass this boundry, which is `index` - 1,
        // and calculate a score for it, and check if it's better than our best so far
        if next_match_last_word_pos - interval_first_match_first_word_pos >= crop_size {
            // if index is 0 there is no last viable match
            if index != 0 {
                let interval_last = index - 1;
                // keep interval if it's the best
                save_best_interval(interval_first, interval_last);
            }

            // advance start of the interval while interval is longer than crop_size.
            loop {
                if interval_first == matches.len() - 1 {
                    break;
                }

                interval_first += 1;
                interval_first_match_first_word_pos = matches[interval_first].get_first_word_pos();

                if interval_first_match_first_word_pos > next_match_last_word_pos
                    || next_match_last_word_pos - interval_first_match_first_word_pos < crop_size
                {
                    break;
                }
            }
        }
    }

    // compute the last interval score and compare it to the best one.
    let interval_last = matches.len() - 1;
    // if it's the last match with itself, we need to make sure it's
    // not a phrase longer than the crop window
    if interval_first != interval_last || matches[interval_first].get_word_count() < crop_size {
        save_best_interval(interval_first, interval_last);
    }

    // if none of the matches fit the criteria above, default to the first one
    best_matches_index_range.map_or([0, 0], |v| v.matches_index_range)
}
