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

# block assigned to a local variable inside another block: lvasgn is not a
# transparent wrapper in RuboCop's in_macro_scope?, so the nested block is
# skipped even with EnforcedStyle: outdent
it "builds a Sinatra app" do
  app = Sinatra.new do
    private
    def priv; end
    public
    def pub; end
  end
end

# rescuing begin breaks macro scope, so access modifiers inside this arbitrary
# block are not bare access modifiers for Layout/AccessModifierIndentation
begin
  namespace :db do
    desc "Setup the database"
    task setup: [:drop]

    private

    def disconnect; end
  end
rescue LoadError
end

# same rescuing-begin wrapper, but with module_function in a nested block
report "loading program" do
  begin
    DidYouMean::JaroWinkler.module_eval do
      module_function
      def distance(str1, str2); end
    end
  rescue LoadError
  end
end
