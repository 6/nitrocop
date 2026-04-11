# nitrocop-config: EnforcedStyle: outdent
# Non-class-constructor block inside a constant assignment: RuboCop's
# bare_access_modifier? returns false because casgn breaks in_macro_scope?
# chain. CommandClass is not Class/Module/Struct/Data, so the block is not
# a class_constructor? and the constant assignment breaks macro scope.
module Authentication
  Authenticate = CommandClass.new(
    dependencies: {},
    inputs: []
  ) do
    def call; end

    private

    def secret; end
  end
end
