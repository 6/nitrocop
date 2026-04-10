puts x.
  merge(
      b: 2
      ^^^^ Layout/FirstArgumentIndentation: Indent the first argument one step more than the start of the previous line.
  )

# Block argument only — wrong indentation relative to receiver
@hash['key'].map(
        &method(:parse)
        ^^^^^^^^^^^^^^^ Layout/FirstArgumentIndentation: Indent the first argument one step more than `@hash['key'].map(`.
