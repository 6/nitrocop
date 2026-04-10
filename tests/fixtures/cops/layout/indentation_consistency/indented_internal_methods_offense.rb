# nitrocop-config: EnforcedStyle: indented_internal_methods

# Block inside a def: protected/private is NOT a bare access modifier
# (not in macro scope), so it does NOT act as a section divider.
# Items at different indentation levels are flagged.
class A
  def test_protected
    @klass.class_eval do
      protected
        def park
        ^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
          true
        end
    end
  end
end

# Inconsistency within a section should still be flagged
class Bar
  private

    def baz
    end

      def qux
      ^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
      end
end
