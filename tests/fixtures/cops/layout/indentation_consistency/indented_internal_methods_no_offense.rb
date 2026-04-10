# nitrocop-config: EnforcedStyle: indented_internal_methods

# In indented_internal_methods, access modifiers are section dividers.
# Methods in each section must be consistent within the section,
# but different sections can have different indentation levels.

class Foo
  def bar
  end

  private

    def baz
    end

    def qux
    end
end

# if body inside a class: private is a bare access modifier (in macro scope),
# so it acts as a section divider. Methods before and after private at
# different indent levels should NOT be flagged.
class MysqlTypeLookupTest
  if true
    def test_one
    end

    def test_two
    end

    private
      def helper_one
      end

      def helper_two
      end
  end
end
