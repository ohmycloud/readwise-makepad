import unittest

from generate_reader_vocabulary import RANK_LIMITS, reading_band


class ReadingBandTests(unittest.TestCase):
    def test_rank_boundaries_and_rarer_words(self):
        for band, limit in enumerate(RANK_LIMITS, 1):
            self.assertEqual(reading_band(str(limit), "0", None), band)
            self.assertEqual(reading_band("0", str(limit), None), band)
            self.assertEqual(reading_band(str(limit + 1), "0", None), band + 1)

    def test_smallest_nonzero_corpus_rank_wins(self):
        self.assertEqual(reading_band("200000", "800", 4), 1)
        self.assertEqual(reading_band("0", "10001", 1), 5)
        self.assertEqual(reading_band("0", "0", None), 7)
        self.assertEqual(reading_band("", "", None), 7)

    def test_legacy_tiers_are_only_a_missing_rank_fallback(self):
        for tier in range(1, 5):
            self.assertEqual(reading_band("0", "0", tier), tier + 1)
        self.assertEqual(reading_band("0", "0", 5), 7)
        self.assertEqual(reading_band("100000", "0", 1), 7)


if __name__ == "__main__":
    unittest.main()
