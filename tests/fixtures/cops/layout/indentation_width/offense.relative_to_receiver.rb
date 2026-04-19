# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver, IndentationStyleEnforced: tabs, IndentationConsistencyStyle: indented_internal_methods, AccessModifierIndentationStyle: outdent, EndAlignmentStyle: variable
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

# Tabs-style block bodies still align from the block base, not the `do` token.
create_table :score_adjustments do |t|
	  t.integer :kind, null: false
^^^ Layout/IndentationWidth: Use 1 (not 2) tabs for indentation.
end

# private def bodies still check indentation under tabs style.
module Outer
	class Inner
		private def helper
			body
^^^ Layout/IndentationWidth: Use 1 (not 0) tabs for indentation.
		end
	end
end

# Sole access modifiers with args are still checked when the class body is not
# wrapped in a begin/statements node.
class ::Class
    public :include
^^^^ Layout/IndentationWidth: Use 1 (not 2) tabs for indentation.
end
