# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver, IndentationStyleEnforced: tabs, EndAlignmentStyle: variable
module Wrapper
  def language_link(language, label = nil)
    language = if language.respond_to? :map
                 language.map(&method(:escape_language))
^^^^^^^^^^^^^^^^^ Layout/IndentationWidth: Use 1 (not 6) tabs for indentation.
               else
                 escape_language language
               end
  end
end

# Mixed leading spaces and tabs still count by raw width under tabs style.
class Test
  def foo
 		bar
^^^ Layout/IndentationWidth: Use 1 (not 1) tabs for indentation.
  end
end

# Multiline conditionals used as the last call argument align from the call.
process(if ready?
          primary_value
^^^^^^^^^^ Layout/IndentationWidth: Use 1 (not 5) tabs for indentation.
        else
          fallback_value
        end)
