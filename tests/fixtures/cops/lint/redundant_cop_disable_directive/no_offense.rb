x = 1
y = 2
z = 3
a = 4
b = 5
c = 6

# Renamed cops whose new name has known detection gaps (in REDUNDANT_DISABLE_SKIP_COPS)
# should not be flagged — the directive might legitimately suppress an offense nitrocop missed.
# rubocop:disable Metrics/LineLength
this_is_a_very_long_line_that_should_trigger_line_length_cop_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa = 1
# rubocop:enable Metrics/LineLength
