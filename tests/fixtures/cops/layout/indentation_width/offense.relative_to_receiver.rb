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
