use std::cmp::Reverse;
use std::fmt::Debug;

use charabia::Token;

use super::super::interner::Interned;
use super::super::query_term::LocatedQueryTerm;
use super::super::{DedupInterner, Phrase};
use super::r#match::{Match, MatchPosition};
use crate::SearchContext;

// TODO: Consider using a tuple here, because indexing this thing out of bounds only incurs a runtime error
pub type UserQueryPositionRange = [u16; 2];

struct LocatedMatchingPhrase {
    value: Interned<Phrase>,
    position: UserQueryPositionRange,
}

struct LocatedMatchingWords {
    value: Vec<Interned<String>>,
    position: UserQueryPositionRange,
    is_prefix: bool,
    original_char_count: usize,
}

struct TokenPositionHelper<'a> {
    token: &'a Token<'a>,
    position_by_word: usize,
    position_by_token: usize,
}

impl<'a> TokenPositionHelper<'a> {
    fn iter_from_tokens(tokens: &'a [Token]) -> impl Iterator<Item = Self> + Clone {
        tokens
            .iter()
            .scan([0, 0], |[token_position, word_position], token| {
                let token_word_thingy = Self {
                    position_by_token: *token_position,
                    position_by_word: *word_position,
                    token,
                };

                *token_position += 1;

                if !token.is_separator() {
                    *word_position += 1;
                }

                Some(token_word_thingy)
            })
            .filter(|t| !t.token.is_separator())
    }
}

/// Structure created from a query tree
/// referencing words that match the given query tree.
#[derive(Default)]
pub struct MatchingWords {
    word_interner: DedupInterner<String>,
    phrase_interner: DedupInterner<Phrase>,
    located_matching_phrases: Vec<LocatedMatchingPhrase>,
    located_matching_words: Vec<LocatedMatchingWords>,
}

impl MatchingWords {
    pub fn new(ctx: SearchContext, located_terms: &[LocatedQueryTerm]) -> Self {
        let mut located_matching_phrases = Vec::new();
        let mut located_matching_words = Vec::new();

        // Extract and centralize the different phrases and words to match stored in a QueryTerm
        // and wrap them in dedicated structures.
        for LocatedQueryTerm { value, positions } in located_terms {
            let term = ctx.term_interner.get(*value);
            let (matching_words, matching_phrases) = term.all_computed_derivations();

            let position = [*positions.start(), *positions.end()];

            located_matching_phrases.reserve(matching_phrases.len());
            located_matching_phrases.extend(matching_phrases.iter().map(|matching_phrase| {
                LocatedMatchingPhrase { value: *matching_phrase, position }
            }));

            located_matching_words.push(LocatedMatchingWords {
                value: matching_words,
                position,
                is_prefix: term.is_prefix(),
                original_char_count: term.original_word(&ctx).chars().count(),
            });
        }

        // Sort words by having `is_prefix` as false first and then by their lengths in reverse order.
        // This is only meant to help with what we match a token against first.
        located_matching_words.sort_unstable_by_key(|lmw| {
            (lmw.is_prefix, Reverse(lmw.position[1] - lmw.position[0]))
        });

        Self {
            located_matching_phrases,
            located_matching_words,
            word_interner: ctx.word_interner,
            phrase_interner: ctx.phrase_interner,
        }
    }

    fn try_get_phrase_match<'a>(
        &self,
        token_position_helper_iter: &mut (impl Iterator<Item = TokenPositionHelper<'a>> + Clone),
    ) -> Option<Match> {
        let mut mapped_phrase_iter = self.located_matching_phrases.iter().map(|lmp| {
            let words_iter = self
                .phrase_interner
                .get(lmp.value)
                .words
                .iter()
                .map(|word_option| word_option.map(|word| self.word_interner.get(word).as_str()))
                .peekable();

            (lmp.position, words_iter)
        });

        'outer: loop {
            let Some((query_position, mut words_iter)) = mapped_phrase_iter.next() else {
                return None;
            };

            // TODO: Is it worth only cloning if we have to?
            let mut tph_iter = token_position_helper_iter.clone();

            let mut first_tph_details = None;
            let last_tph_details = loop {
                // 1. get word from `words_iter` and token word thingy from `token_word_thingy_iter`
                let (Some(word), Some(tph)) = (words_iter.next(), tph_iter.next()) else {
                    // 2. if there are no more words or token word thingys, get to next phrase and reset `token_word_thingy_iter`
                    continue 'outer;
                };

                // ?. save first token position bla bla bla
                if first_tph_details.is_none() {
                    first_tph_details = Some([
                        tph.position_by_token,
                        tph.position_by_word,
                        tph.token.char_start,
                        tph.token.byte_start,
                    ]);
                }

                // 3. check if word matches our token
                let is_matching = match word {
                    Some(word) => tph.token.lemma() == word,
                    // a `None` value in the phrase words iterator corresponds to a stop word,
                    // the value is considered a match if the current token is categorized as a stop word.
                    None => tph.token.is_stopword(),
                };

                // 4. if it does not, get to next phrase and restart `token_word_thingy_iter`
                if !is_matching {
                    continue 'outer;
                }

                // 5. if it does, and there are no words left, time to return
                if words_iter.peek().is_none() {
                    break [
                        tph.position_by_token,
                        tph.position_by_word,
                        tph.token.char_end,
                        tph.token.byte_end,
                    ];
                }
            };

            let Some(
                [first_tph_position_by_token, first_tph_position_by_word, first_tph_char_start, first_tph_byte_start],
            ) = first_tph_details
            else {
                panic!("TODO");
            };
            let [last_tph_position_by_token, last_tph_position_by_word, last_tph_char_end, last_tph_byte_end] =
                last_tph_details;

            *token_position_helper_iter = tph_iter;

            return Some(Match::Phrase {
                byte_len: last_tph_byte_end - first_tph_byte_start + 1,
                char_count: last_tph_char_end - first_tph_char_start + 1,
                word_position_range: [first_tph_position_by_word, last_tph_position_by_word],
                token_position_range: [first_tph_position_by_token, last_tph_position_by_token],
                query_position_range: query_position,
            });
        }
    }

    /// Try to match the token with one of the located_words.
    fn try_get_word_match(&self, tph: TokenPositionHelper) -> Option<Match> {
        let mut iter =
            self.located_matching_words.iter().flat_map(|lw| lw.value.iter().map(move |w| (lw, w)));

        loop {
            let (located_words, word) = iter.next()?;
            let word = self.word_interner.get(*word);

            let [char_count, byte_len] =
                if located_words.is_prefix && tph.token.lemma().starts_with(word) {
                    // TODO: Isn't there something on [Token] already for this, or something that makes this simpler?
                    // if the word is a prefix we match using starts_with
                    let Some(prefix_byte_len) = word
                        .char_indices()
                        .nth(located_words.original_char_count - 1)
                        .map(|(char_index, c)| char_index + c.len_utf8())
                    else {
                        continue;
                    };

                    [located_words.original_char_count, prefix_byte_len]
                } else if tph.token.lemma() == word {
                    // else if we exact, match the token
                    [
                        tph.token.char_end - tph.token.char_start + 1,
                        tph.token.byte_end - tph.token.byte_start + 1,
                    ]
                } else {
                    continue;
                };

            return Some(Match {
                char_count,
                byte_len,
                position: MatchPosition::Word {
                    word_position: tph.position_by_word,
                    token_position: tph.position_by_token,
                },
                query_position_range: located_words.position,
            });
        }
    }

    pub fn get_matches(&self, tokens: &[Token]) -> Vec<Match> {
        let mut token_position_helper_iter = TokenPositionHelper::iter_from_tokens(tokens);
        let mut matches = Vec::new();

        loop {
            // try and get a phrase match
            if let Some(r#match) = self.try_get_phrase_match(&mut token_position_helper_iter) {
                matches.push(r#match);

                continue;
            }

            // if the above fails, try get next token position helper
            if let Some(tph) = token_position_helper_iter.next() {
                // and then try and get a word match
                if let Some(r#match) = self.try_get_word_match(tph) {
                    matches.push(r#match);
                }
            } else {
                // there are no more items in the iterator, we are done searching for matches
                break;
            };
        }

        // TODO: Explain why
        matches.sort_unstable_by(|a, b| a.query_position_range[0].cmp(&b.query_position_range[0]));

        matches
    }
}

impl Debug for MatchingWords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let MatchingWords {
            word_interner,
            phrase_interner,
            located_matching_phrases: phrases,
            located_matching_words: words,
        } = self;

        let phrases: Vec<_> = phrases
            .iter()
            .map(|p| {
                (
                    phrase_interner
                        .get(p.value)
                        .words
                        .iter()
                        .map(|w| w.map_or("STOP_WORD", |w| word_interner.get(w)))
                        .collect::<Vec<_>>()
                        .join(" "),
                    p.position.clone(),
                )
            })
            .collect();

        let words: Vec<_> = words
            .iter()
            .flat_map(|w| {
                w.value
                    .iter()
                    .map(|s| (word_interner.get(*s), w.position.clone(), w.is_prefix))
                    .collect::<Vec<_>>()
            })
            .collect();

        f.debug_struct("MatchingWords").field("phrases", &phrases).field("words", &words).finish()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::borrow::Cow;

    use charabia::{TokenKind, TokenizerBuilder};

    use super::super::super::located_query_terms_from_tokens;
    use super::*;
    use crate::index::tests::TempIndex;
    use crate::search::new::query_term::ExtractedTokens;

    pub(crate) fn temp_index_with_documents() -> TempIndex {
        let temp_index = TempIndex::new();
        temp_index
            .add_documents(documents!([
                { "id": 1, "name": "split this world westfali westfalia the Ŵôřlḑôle" },
                { "id": 2, "name": "Westfália" },
                { "id": 3, "name": "Ŵôřlḑôle" },
            ]))
            .unwrap();
        temp_index
    }

    #[test]
    fn matching_words() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let mut ctx = SearchContext::new(&temp_index, &rtxn).unwrap();
        let mut builder = TokenizerBuilder::default();
        let tokenizer = builder.build();
        let tokens = tokenizer.tokenize("split this world");
        let ExtractedTokens { query_terms, .. } =
            located_query_terms_from_tokens(&mut ctx, tokens, None).unwrap();
        let matching_words = MatchingWords::new(ctx, &query_terms);

        assert_eq!(
            matching_words
                .match_token(&Token {
                    kind: TokenKind::Word,
                    lemma: Cow::Borrowed("split"),
                    char_end: "split".chars().count(),
                    byte_end: "split".len(),
                    ..Default::default()
                })
                .next(),
            Some(MatchType::Complete {
                details: CompleteMatch::Full { char_count: 5, byte_len: 5 },
                position: [0, 0]
            })
        );
        assert_eq!(
            matching_words
                .match_token(&Token {
                    kind: TokenKind::Word,
                    lemma: Cow::Borrowed("nyc"),
                    char_end: "nyc".chars().count(),
                    byte_end: "nyc".len(),
                    ..Default::default()
                })
                .next(),
            None
        );
        assert_eq!(
            matching_words
                .match_token(&Token {
                    kind: TokenKind::Word,
                    lemma: Cow::Borrowed("world"),
                    char_end: "world".chars().count(),
                    byte_end: "world".len(),
                    ..Default::default()
                })
                .next(),
            Some(MatchType::Complete {
                details: CompleteMatch::Full { char_count: 5, byte_len: 5 },
                position: [2, 2]
            })
        );
        assert_eq!(
            matching_words
                .match_token(&Token {
                    kind: TokenKind::Word,
                    lemma: Cow::Borrowed("worlded"),
                    char_end: "worlded".chars().count(),
                    byte_end: "worlded".len(),
                    ..Default::default()
                })
                .next(),
            Some(MatchType::Complete {
                details: CompleteMatch::Full { char_count: 5, byte_len: 5 },
                position: [2, 2]
            })
        );
        assert_eq!(
            matching_words
                .match_token(&Token {
                    kind: TokenKind::Word,
                    lemma: Cow::Borrowed("thisnew"),
                    char_end: "thisnew".chars().count(),
                    byte_end: "thisnew".len(),
                    ..Default::default()
                })
                .next(),
            None
        );
    }
}
