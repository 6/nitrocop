# nitrocop-config: EnforcedStyle: indented

def message
  authn.message(
    success_msg:
      "a " \
        "b",
        ^^^ Layout/LineEndStringConcatenationIndentation: Indent the first part of a string concatenated with backslash.
    error_msg: error_message
  )
end

MESSAGES = { KeyAlignment => 'Align the keys of a hash literal if ' \
                             'they span more than one line.' }
                             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineEndStringConcatenationIndentation: Indent the first part of a string concatenated with backslash.
