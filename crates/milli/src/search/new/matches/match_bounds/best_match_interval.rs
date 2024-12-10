use super::super::matching_words::WordId;
use super::super::{Match, MatchPosition};

struct MatchIntervalWithScore {
    interval: [usize; 2],
    score: [i16; 3],
}

// count score for phrases
fn tally_phrase_scores(fwp: usize, lwp: usize, order_score: &mut i16, distance_score: &mut i16) {
    let words_in_phrase_minus_one = (lwp - fwp) as i16;
    // will always be ordered, so +1 for each space between words
    *order_score += words_in_phrase_minus_one;
    // distance will always be 1, so -1 for each space between words
    *distance_score -= words_in_phrase_minus_one;
}

// TODO: If matches are ordered, this can only ever grow right
// UPDATE: they're not necessarily ordered; first come phrases, then words, then prefix word matches
// QUESTION: does this order even matter where it's defined? Should we get rid of it?
enum Changed {
    Left,
    Right
}

/// Compute the score of a match interval:
/// 1) count unique matches
/// 2) calculate distance between matches
/// 3) count ordered matches
fn get_interval_score(matches: &[Match]) -> [i16; 3] {
    let mut ids: Vec<[WordId; 2]> = Vec::with_capacity(matches.len());
    let mut order_score = 0;
    let mut distance_score = 0;

    let mut iter = matches.iter().peekable();
    while let Some(m) = iter.next() {
        let [m_id_first, m_id_last] = m.ids;

        if let Some(next_match) = iter.peek() {
            let [n_m_id_first, n_m_id_last] = next_match.ids;
            // if matches are ordered
            if n_m_id_first > m_id_first {
                order_score += 1;
            }

            let m_last_word_pos = match m.position {
                MatchPosition::Word { word_position, .. } => word_position,
                MatchPosition::Phrase { word_positions: [fwp, lwp], .. } => {
                    tally_phrase_scores(fwp, lwp, &mut order_score, &mut distance_score);
                    lwp
                }
            };
            let next_match_first_word_pos = next_match.get_first_word_pos();

            // compute distance between matches
            distance_score -= (next_match_first_word_pos - m_last_word_pos).min(7) as i16;
        } else if let MatchPosition::Phrase { word_positions: [fwp, lwp], .. } = m.position {
            // in case last match is a phrase, count score for its words
            tally_phrase_scores(fwp, lwp, &mut order_score, &mut distance_score);
        }

        let mut changed = None;
        let mut ind = 0;
        while ind != ids.len() - 1 {
            let ids_ind = &mut ids[ind];
            let [id_first, id_last] = *ids_ind;

            if m_id_last >= id_first && m_id_first <= id_last {
                if m_id_first < id_first {
                    ids_ind[0] = m_id_first;
                    changed = Some(Changed::Left);
                }

                if m_id_last > id_last {
                    ids_ind[1] = m_id_last;
                    changed = Some(Changed::Right);
                }
            }

            // TODO: IF anything changed above in any of the directions, check if any of the other elements can be folded

            ind += 1;
        }

        ids.push(m.ids);
    }

    ids.sort_unstable();
    ids.dedup();
    let unique_score = ids.len() as i16;

    // rank by unique match count, then by distance between matches, then by ordered match count.
    [unique_score, distance_score, order_score]
}

/// Returns the first and last match where the score computed by match_interval_score is the best.
pub fn get_best_match_interval(matches: &[Match], crop_size: usize) -> [usize; 2] {
    // positions of the first and the last match of the best matches interval in `matches`.
    let mut best_interval = None;

    let mut save_best_interval = |interval_first, interval_last| {
        let interval_score = get_interval_score(&matches[interval_first..=interval_last]);
        let is_interval_score_better = best_interval
            .as_ref()
            .map_or(true, |MatchIntervalWithScore { score, .. }| interval_score > *score);

        if is_interval_score_better {
            best_interval = Some(MatchIntervalWithScore {
                interval: [interval_first, interval_last],
                score: interval_score,
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
    best_interval
        .map_or([0, 0], |MatchIntervalWithScore { interval: [first, last], .. }| [first, last])
}
