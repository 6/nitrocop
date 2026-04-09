x = 1
y = 2
z = 3
a = 4
b = 5
c = 6

# Renamed cop (Metrics/LineLength → Layout/LineLength) with an offense on the
# disabled line — the directive suppresses a real offense, so it is not redundant.
# rubocop:disable Metrics/LineLength
this_is_a_very_long_line_that_should_trigger_line_length_cop_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa = 1
# rubocop:enable Metrics/LineLength

# FP fix: malformed cop name (/BlockLength) — not a valid cop name, ignore silently
# rubocop:disable /BlockLength, Metrics/
x = 1
# rubocop:enable /BlockLength, Metrics/
