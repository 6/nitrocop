x = "hello"
y = "world"
z = "interpolated #{value}"
a = 'contains "double" quotes'
b = "newline\n"

# Single-quoted strings where \\ is followed by a non-safe char — converting
# to double quotes would turn the literal two chars into an escape sequence.
# RuboCop's \\[^'\\] regex catches these without pairing backslashes.
c = '\\d+'
d = '\\n'
e = '\\x41'
f = 'type %SystemDrive%\\\\boot.ini'
