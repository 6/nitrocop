# nitrocop-config: EnforcedStyle: indented
def lyrics
  "#{bottle_number} of beer on the wall, ".capitalize +
  "#{bottle_number} of beer.\n" +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "#{bottle_number.action}, " +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "#{bottle_number.successor} of beer on the wall.\n"
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end
