require File.expand_path( File.dirname( __FILE__ ) ) + '/lib/arachni'
                                                  ^ Layout/SpaceInsideParens: Space inside parentheses detected.

g( ( x ))
  ^ Layout/SpaceInsideParens: Space inside parentheses detected.

warning(%(
        ^ Layout/SpaceInsideParens: No space inside parentheses detected.
  x
))
 ^ Layout/SpaceInsideParens: No space inside parentheses detected.
