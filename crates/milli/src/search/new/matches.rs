mod r#match;
mod match_bounds;
mod matching_words;

use charabia::{Language, Token, Tokenizer};
pub use match_bounds::MatchBounds;
pub use matching_words::MatchingWords;
use r#match::Match;
use std::borrow::Cow;

const DEFAULT_CROP_MARKER: &str = "…";
const DEFAULT_HIGHLIGHT_PREFIX: &str = "<em>";
const DEFAULT_HIGHLIGHT_SUFFIX: &str = "</em>";

/// Structure used to build a Matcher allowing to customize formating tags.
pub struct MatcherBuilder<'m> {
    matching_words: MatchingWords,
    tokenizer: Tokenizer<'m>,
    crop_marker: Option<String>,
    highlight_prefix: Option<String>,
    highlight_suffix: Option<String>,
}

impl<'m> MatcherBuilder<'m> {
    pub fn new(matching_words: MatchingWords, tokenizer: Tokenizer<'m>) -> Self {
        Self {
            matching_words,
            tokenizer,
            crop_marker: None,
            highlight_prefix: None,
            highlight_suffix: None,
        }
    }

    pub fn crop_marker(&mut self, marker: String) -> &Self {
        self.crop_marker = Some(marker);
        self
    }

    pub fn highlight_prefix(&mut self, prefix: String) -> &Self {
        self.highlight_prefix = Some(prefix);
        self
    }

    pub fn highlight_suffix(&mut self, suffix: String) -> &Self {
        self.highlight_suffix = Some(suffix);
        self
    }

    pub fn build<'t, 'lang>(
        &self,
        text: &'t str,
        locales: Option<&'lang [Language]>,
    ) -> Matcher<'t, 'm, '_, 'lang> {
        Matcher {
            text,
            matching_words: &self.matching_words,
            tokenizer: &self.tokenizer,
            crop_marker: self.crop_marker.as_ref().map_or(DEFAULT_CROP_MARKER, |v| v.as_str()),
            highlight_prefix: self
                .highlight_prefix
                .as_ref()
                .map_or(DEFAULT_HIGHLIGHT_PREFIX, |v| v.as_str()),
            highlight_suffix: self
                .highlight_suffix
                .as_ref()
                .map_or(DEFAULT_HIGHLIGHT_SUFFIX, |v| v.as_str()),
            tokens_and_matches: None,
            locales,
        }
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct FormatOptions {
    pub highlight: bool,
    pub crop: Option<usize>,
}

impl FormatOptions {
    pub fn merge(self, other: Self) -> Self {
        Self { highlight: self.highlight || other.highlight, crop: self.crop.or(other.crop) }
    }

    pub fn should_format(&self) -> bool {
        self.highlight || self.crop.is_some()
    }
}

/// Structure used to analyze a string, compute words that match,
/// and format the source string, returning a highlighted and cropped sub-string.
pub struct Matcher<'t, 'tokenizer, 'b, 'lang> {
    text: &'t str,
    matching_words: &'b MatchingWords,
    tokenizer: &'b Tokenizer<'tokenizer>,
    locales: Option<&'lang [Language]>,
    crop_marker: &'b str,
    highlight_prefix: &'b str,
    highlight_suffix: &'b str,
    tokens_and_matches: Option<(Vec<Token<'t>>, Vec<Match>)>,
}

impl<'t> Matcher<'t, '_, '_, '_> {
    /// TODO: description
    pub fn get_match_bounds(&mut self, format_options: FormatOptions) -> MatchBounds {
        if self.text.is_empty() {
            return MatchBounds::Full;
        }

        let (tokens, matches) = self.tokens_and_matches.get_or_insert_with(|| {
            // lazily get tokens and compute matches
            let tokens = self
                .tokenizer
                .tokenize_with_allow_list(self.text, self.locales)
                .collect::<Vec<_>>();

            let matches = self.matching_words.get_matches(&tokens);

            (tokens, matches)
        });

        match_bounds::get_match_bounds(tokens, matches, format_options)
    }

    // Returns the formatted version of the original text.
    pub fn get_formatted_text(&mut self, format_options: FormatOptions) -> Cow<'t, str> {
        if !format_options.highlight && format_options.crop.is_none() {
            // compute matches is not needed if no highlight nor crop is requested
            return Cow::Borrowed(self.text);
        }

        let (first, indexes) = match self.get_match_bounds(format_options) {
            MatchBounds::Full => {
                return Cow::Borrowed(self.text);
            }
            MatchBounds::Formatted { highlight_toggle: first, indexes } => (first, indexes),
        };

        let mut should_be_highlighted = first;
        let mut formatted = Vec::new();

        let mut previous_index = &indexes[0];
        let indexes_iter = indexes.iter().skip(1);

        // push crop marker if it's not the start of the text
        if !self.crop_marker.is_empty() && *previous_index != 0 {
            formatted.push(self.crop_marker);
        }

        for index in indexes_iter {
            if should_be_highlighted {
                formatted.push(self.highlight_prefix);
            }

            formatted.push(&self.text[*previous_index..*index]);

            if should_be_highlighted {
                formatted.push(self.highlight_suffix);
            }

            should_be_highlighted = !should_be_highlighted;
            previous_index = index;
        }

        // push crop marker if it's not the end of the text
        if !self.crop_marker.is_empty() && *previous_index < self.text.len() {
            formatted.push(self.crop_marker);
        }

        if formatted.len() == 1 {
            // avoid concatenating if there is only one element
            return Cow::Owned(formatted[0].to_string());
        }

        Cow::Owned(formatted.concat())
    }
}

#[cfg(test)]
mod tests {
    use charabia::TokenizerBuilder;
    use matching_words::tests::temp_index_with_documents;

    use super::*;
    use crate::index::tests::TempIndex;
    use crate::{execute_search, filtered_universe, SearchContext, TimeBudget};

    impl<'a> MatcherBuilder<'a> {
        fn new_test(rtxn: &'a heed::RoTxn<'a>, index: &'a TempIndex, query: &str) -> Self {
            let mut ctx = SearchContext::new(index, rtxn).unwrap();
            let universe = filtered_universe(ctx.index, ctx.txn, &None).unwrap();
            let crate::search::PartialSearchResult { located_query_terms, .. } = execute_search(
                &mut ctx,
                Some(query),
                crate::TermsMatchingStrategy::default(),
                crate::score_details::ScoringStrategy::Skip,
                false,
                universe,
                &None,
                &None,
                crate::search::new::GeoSortStrategy::default(),
                0,
                100,
                Some(10),
                &mut crate::DefaultSearchLogger,
                &mut crate::DefaultSearchLogger,
                TimeBudget::max(),
                None,
                None,
            )
            .unwrap();

            // consume context and located_query_terms to build MatchingWords.
            let matching_words = match located_query_terms {
                Some(located_query_terms) => MatchingWords::new(ctx, &located_query_terms),
                None => MatchingWords::default(),
            };

            MatcherBuilder::new(matching_words, TokenizerBuilder::default().into_tokenizer())
        }
    }

    #[test]
    fn format_identity() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "split the world");

        let format_options = FormatOptions { highlight: false, crop: None };

        // Text without any match.
        let text = "A quick brown fox can not jump 32 feet, right? Brr, it is cold!";
        let mut matcher = builder.build(text, None);
        // no crop and no highlight should return complete text.
        assert_eq!(&matcher.get_formatted_text(format_options), &text);

        // Text containing all matches.
        let text = "Natalie risk her future to build a world with the boy she loves. Emily Henry: The Love That Split The World.";
        let mut matcher = builder.build(text, None);
        // no crop and no highlight should return complete text.
        assert_eq!(&matcher.get_formatted_text(format_options), &text);

        // Text containing some matches.
        let text = "Natalie risk her future to build a world with the boy she loves.";
        let mut matcher = builder.build(text, None);
        // no crop and no highlight should return complete text.
        assert_eq!(&matcher.get_formatted_text(format_options), &text);
    }

    #[test]
    fn format_highlight() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "split the world");

        let format_options = FormatOptions { highlight: true, crop: None };

        // empty text.
        let text = "";
        let mut matcher = builder.build(text, None);
        assert_eq!(&matcher.get_formatted_text(format_options), "");

        // text containing only separators.
        let text = ":-)";
        let mut matcher = builder.build(text, None);
        assert_eq!(&matcher.get_formatted_text(format_options), ":-)");

        // Text without any match.
        let text = "A quick brown fox can not jump 32 feet, right? Brr, it is cold!";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text, because there is no matches.
        assert_eq!(&matcher.get_formatted_text(format_options), &text);

        // Text containing all matches.
        let text = "Natalie risk her future to build a world with the boy she loves. Emily Henry: The Love That Split The World.";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text with highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"Natalie risk her future to build a <em>world</em> with <em>the</em> boy she loves. Emily Henry: <em>The</em> Love That <em>Split</em> <em>The</em> <em>World</em>."
        );

        // Text containing some matches.
        let text = "Natalie risk her future to build a world with the boy she loves.";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text with highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"Natalie risk her future to build a <em>world</em> with <em>the</em> boy she loves."
        );
    }

    #[test]
    fn highlight_unicode() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "world");
        let format_options = FormatOptions { highlight: true, crop: None };

        // Text containing prefix match.
        let text = "Ŵôřlḑôle";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text with highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"<em>Ŵôřlḑ</em>ôle"
        );

        // Text containing unicode match.
        let text = "Ŵôřlḑ";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text with highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"<em>Ŵôřlḑ</em>"
        );

        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "westfali");
        let format_options = FormatOptions { highlight: true, crop: None };

        // Text containing unicode match.
        let text = "Westfália";
        let mut matcher = builder.build(text, None);
        // no crop should return complete text with highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"<em>Westfáli</em>a"
        );
    }

    #[test]
    fn format_crop() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "split the world");

        let format_options = FormatOptions { highlight: false, crop: Some(10) };

        // empty text.
        let text = "";
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @""
        );

        // text containing only separators.
        let text = ":-)";
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @":-)"
        );

        // Text without any match.
        let text = "A quick brown fox can not jump 32 feet, right? Brr, it is cold!";
        let mut matcher = builder.build(text, None);
        // no highlight should return 10 first words with a marker at the end.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"A quick brown fox can not jump 32 feet, right…"
        );

        // Text without any match starting by a separator.
        let text = "(A quick brown fox can not jump 32 feet, right? Brr, it is cold!)";
        let mut matcher = builder.build(text, None);
        // no highlight should return 10 first words with a marker at the end.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"(A quick brown fox can not jump 32 feet, right…"
        );

        // Test phrase propagation
        let text = "Natalie risk her future. Split The World is a book written by Emily Henry. I never read it.";
        let mut matcher = builder.build(text, None);
        // should crop the phrase instead of croping around the match.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…Split The World is a book written by Emily Henry…"
        );

        // Text containing some matches.
        let text = "Natalie risk her future to build a world with the boy she loves.";
        let mut matcher = builder.build(text, None);
        // no highlight should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…future to build a world with the boy she loves."
        );

        // Text containing all matches.
        let text = "Natalie risk her future to build a world with the boy she loves. Emily Henry: The Love That Split The World.";
        let mut matcher = builder.build(text, None);
        // no highlight should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…she loves. Emily Henry: The Love That Split The World."
        );

        // Text containing a match unordered and a match ordered.
        let text = "The world split void void void void void void void void void split the world void void";
        let mut matcher = builder.build(text, None);
        // crop should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…void void void void void split the world void void"
        );

        // Text containing matches with different density.
        let text = "split void the void void world void void void void void void void void void void split the world void void";
        let mut matcher = builder.build(text, None);
        // crop should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…void void void void void split the world void void"
        );

        // Text containing matches with same word.
        let text = "split split split split split split void void void void void void void void void void split the world void void";
        let mut matcher = builder.build(text, None);
        // crop should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…void void void void void split the world void void"
        );
    }

    #[test]
    fn format_highlight_crop() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "split the world");

        let format_options = FormatOptions { highlight: true, crop: Some(10) };

        // empty text.
        let text = "";
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @""
        );

        // text containing only separators.
        let text = ":-)";
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @":-)"
        );

        // Text without any match.
        let text = "A quick brown fox can not jump 32 feet, right? Brr, it is cold!";
        let mut matcher = builder.build(text, None);
        // both should return 10 first words with a marker at the end.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"A quick brown fox can not jump 32 feet, right…"
        );

        // Text containing some matches.
        let text = "Natalie risk her future to build a world with the boy she loves.";
        let mut matcher = builder.build(text, None);
        // both should return 10 last words with a marker at the start and highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…future to build a <em>world</em> with <em>the</em> boy she loves."
        );

        // Text containing all matches.
        let text = "Natalie risk her future to build a world with the boy she loves. Emily Henry: The Love That Split The World.";
        let mut matcher = builder.build(text, None);
        // both should return 10 last words with a marker at the start and highlighted matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…she loves. Emily Henry: <em>The</em> Love That <em>Split</em> <em>The</em> <em>World</em>."
        );

        // Text containing a match unordered and a match ordered.
        let text = "The world split void void void void void void void void void split the world void void";
        let mut matcher = builder.build(text, None);
        // crop should return 10 last words with a marker at the start.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…void void void void void <em>split</em> <em>the</em> <em>world</em> void void"
        );
    }

    #[test]
    fn format_highlight_crop_phrase_query() {
        //! testing: https://github.com/meilisearch/meilisearch/issues/3975
        let temp_index = TempIndex::new();

        let text = "The groundbreaking invention had the power to split the world between those who embraced progress and those who resisted change!";
        temp_index
            .add_documents(documents!([
                { "id": 1, "text": text }
            ]))
            .unwrap();

        let rtxn = temp_index.read_txn().unwrap();

        let format_options = FormatOptions { highlight: true, crop: Some(10) };

        // let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "\"the world\"");
        // let mut matcher = builder.build(text, None);
        // // should return 10 words with a marker at the start as well the end, and the highlighted matches.
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     @"…the power to split <em>the world</em> between those who embraced…"
        // );

        // let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "those \"and those\"");
        // let mut matcher = builder.build(text, None);
        // // should highlight "those" and the phrase "and those".
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     @"…world between <em>those</em> who embraced progress <em>and those</em> who resisted…"
        // );

        // let builder = MatcherBuilder::new_test(
        //     &rtxn,
        //     &temp_index,
        //     "\"The groundbreaking invention had the power to split the world\"",
        // );
        // let mut matcher = builder.build(text, None);
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     @"<em>The groundbreaking invention had the power to split the world</em>…"
        // );

        // let builder = MatcherBuilder::new_test(
        //     &rtxn,
        //     &temp_index,
        //     "\"The groundbreaking invention had the power to split the world between those\"",
        // );
        // let mut matcher = builder.build(text, None);
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     @"<em>The groundbreaking invention had the power to split the world</em>…"
        // );

        // let builder = MatcherBuilder::new_test(
        //     &rtxn,
        //     &temp_index,
        //     "\"The groundbreaking invention\" \"embraced progress and those who resisted change!\"",
        // );
        // let mut matcher = builder.build(text, None);
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     // TODO: Should include exclamation mark without crop markers
        //     @"…between those who <em>embraced progress and those who resisted change</em>!"
        // );

        // let builder = MatcherBuilder::new_test(
        //     &rtxn,
        //     &temp_index,
        //     "\"groundbreaking invention\" \"split the world between\"",
        // );
        // let mut matcher = builder.build(text, None);
        // insta::assert_snapshot!(
        //     matcher.get_formatted_text(format_options),
        //     @"…<em>groundbreaking invention</em> had the power to <em>split the world between</em>…"
        // );

        let builder = MatcherBuilder::new_test(
            &rtxn,
            &temp_index,
            "\"groundbreaking invention\" \"had the power to split the world between those\"",
        );
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…<em>invention</em> <em>had the power to split the world between those</em>…"
        );
    }

    #[test]
    fn smaller_crop_size() {
        //! testing: https://github.com/meilisearch/specifications/pull/120#discussion_r836536295
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "split the world");

        let text = "void void split the world void void.";

        // set a smaller crop size
        let format_options = FormatOptions { highlight: false, crop: Some(2) };
        let mut matcher = builder.build(text, None);
        // because crop size < query size, partially format matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…split the…"
        );

        // set a smaller crop size
        let format_options = FormatOptions { highlight: false, crop: Some(1) };
        let mut matcher = builder.build(text, None);
        // because crop size < query size, partially format matches.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"…split…"
        );

        // set  crop size to 0
        let format_options = FormatOptions { highlight: false, crop: Some(0) };
        let mut matcher = builder.build(text, None);
        // because crop size is 0, crop is ignored.
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"void void split the world void void."
        );
    }

    #[test]
    fn partial_matches() {
        let temp_index = temp_index_with_documents();
        let rtxn = temp_index.read_txn().unwrap();
        let builder = MatcherBuilder::new_test(&rtxn, &temp_index, "the \"t he\" door \"do or\"");

        let format_options = FormatOptions { highlight: true, crop: None };

        let text = "the do or die can't be he do and or isn't he";
        let mut matcher = builder.build(text, None);
        insta::assert_snapshot!(
            matcher.get_formatted_text(format_options),
            @"<em>the</em> <em>do or</em> die can't be he do and or isn'<em>t he</em>"
        );
    }
}
