# nitrocop-config: EnforcedOctalStyle: zero_only
# 0o prefix with octal digits containing underscores should NOT be flagged
# because RuboCop's OCTAL_ZERO_ONLY_REGEX (/^0[Oo][0-7]+$/) requires
# all characters after the prefix to be octal digits only (no underscores).

num = 0o100_666
num = 0o100_644
num = 0o1_000