# nitrocop-config: EnforcedStyle: compact
expect(helper.preview_sizes_for_select).to match_array([
                                                       ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
  ["Phone (360px)", 360],
  ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
                       ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
  ["Small Tablet (640px)", 640]
  ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
                              ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
      ])
      ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.

def css_classes
  [
    ingredient.size ? "h#{ingredient.size}" : nil,
    html_options[:class]
  ]
  ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
end
