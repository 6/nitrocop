class Foo
  def bar
  end

  private

  def baz
  end

  protected

  def qux
  end
end

# Access modifier right after class opening (no blank needed before)
class Bar
  private

  def secret
  end
end

# Access modifier right before end (no blank needed after)
class Baz
  def stuff
  end

  private
end

# Comment before modifier counts as separator
class Qux
  def stuff
  end

  # These methods are private
  private

  def secret
  end
end

# Access modifier as first statement in a block body (no blank needed before)
Class.new do
  private

  def secret
  end
end

# Struct with block
Struct.new("Post", :title) do
  private

  def secret
  end
end

# `private` used as a hash value (not an access modifier)
class Message
  def webhook_data
    {
      message_type: message_type,
      private: private,
      sender: sender
    }
  end

  private

  def secret
  end
end

# `private` used inside a method body conditional (not an access modifier)
class Conversation
  def update_status
    if waiting_present && !private
      clear_waiting
    end
  end

  private

  def clear_waiting
  end
end

# Access modifiers inside a block body are checked like class bodies
app = Sinatra.new do
  private

  def priv; end

  public

  def pub; end
end

# `private` inside a method body (not a bare access modifier)
class Worker
  def private?
    private
  end
end

# `private` deep inside a method body conditional
class Processor
  def private?
    if true
      private
    end
  end
end

# Multiline class definition with superclass on next line
class Controller <
      Base
  private

  def action
  end
end

# Multiline class with sclass on next line
class <<
      self
  private

  def action
  end
end

# Access modifier with trailing comment (blank lines present)
class Config
  def setup
  end

  private # internal helpers

  def helper
  end
end

# Access modifier inside a block at the beginning (no blank needed before)
included do
  private

  def test; end
end

# Access modifier inside a block with arguments
included do |foo|
  private

  def test; end
end

# Access modifier inside a brace block at the beginning
included {
  private

  def test; end
}

# Access modifier inside a nested block
ActiveSupport.on_load(:active_storage_attachment) do
  private

  def secret
  end
end

# Nested DSL block inside another block should not be treated as a visibility scope
it "builds a Sinatra app" do
  app = Sinatra.new do
    private
    def priv; end
    public
    def pub; end
  end
end

# A comment before a top-level access modifier counts as a separator
# comment
private

def top_level_helper
end

# `public` used as an RSpec let variable receiver — not an access modifier
context "user not signed in" do
  context "given a public post" do
    let(:public) { post(:status_message, text: "hello", public: true) }

    it "shows a public post" do
      get :show, params: {id: public.id}
    end
  end
end

# `public` used as hash value in argument list — not an access modifier
describe "messages" do
  let(:public) { recipient.nil? }
  let(:local_parent) { create(:status_message, author: person, public: public) }
  let(:remote_parent) { create(:status_message, author: other.person, public: public) }
end

# `private` inside a single-line inline class — same line as sibling
it "does not delegate private methods" do
  object = Class.new{ private; def hello_world; end }.new
end

# `private` inside single-line class_eval — same line as sibling
it "returns true for own private methods" do
  Decorator.class_eval{private; def hello_world; end}
end

# `private` inside a class_eval block wrapped in `and` — RuboCop's
# `in_macro_scope?` does not recognize `and`/`or` as valid wrappers,
# so `bare_access_modifier?` returns false and the cop does not fire.
defined?(PTY) and defined?(IO.console) and TestIO_Console.class_eval do
  def test_something
  end

  private
  def helper
  end
end

# A bare `public` used as a method argument inside a nested block is not
# an access modifier.
shared_examples_for "copies public attribute" do
  describe "#public" do
    context "when not public" do
      let(:public) { false }

      it "copies correctly" do
        expect(subject).to be_success
        expect(project_copy.public).to eq public
      end
    end
  end
end

# A bare `public` used as an `if` condition inside a nested block is not
# an access modifier.
shared_examples_for "messages which are indifferent about sharing fact" do
  let(:public) { recipient.nil? }

  it "treats status message receive correctly" do
    entity = Fabricate(:status_message_entity, author: sender_id, public: public)

    if public
      expect(Diaspora::Federation::Dispatcher).to receive(:build) do |_user, participation, _opts|
        expect(participation.target.guid).to eq(entity.guid)
        instance_double(:dispatch)
      end
    else
      expect(Diaspora::Federation::Dispatcher).not_to receive(:build)
    end
  end
end

# A proc literal on the previous line updates RuboCop's block-opening state,
# so no extra blank line is required before `private`.
class ExpressionFileInfo
  validate :unset_units_unless_raw_counts
  validate :enforce_units_on_raw_counts
  validate :enforce_raw_counts_associations, unless: proc { |attributes| attributes[:is_raw_counts] }
  private

  def sanitize_raw_counts_associations
  end
end

# A preceding inline class updates RuboCop's class-opening state, so no extra
# blank line is required before `module_function`.
module PdfFill
  module Filler
    class PdfFillerException < StandardError; end
    module_function

    STATSD_KEY_PREFIX = "api.pdf_fill"
  end
end

# `case` / `when` wrappers do not propagate macro scope into receiverful blocks.
case adapter
when "oracle"
  ActiveRecord::ConnectionAdapters::OracleEnhancedAdapter.class_eval do
    def quoted_date(value)
      value.to_s(:db)
    end
    private
    # PATCH: Restore previous mapping of ActiveRecord datetime to DATE type.
    const_get(:NATIVE_DATABASE_TYPES)[:datetime] = {name: "DATE"}
  end
end

# `case` / `when` also suppress access-modifier handling at the start of the block.
case adapter
when "sqlserver"
  ::Arel::Visitors::ToSql.class_eval do
    private
    def visit_Arel_Nodes_Limit(o, collector)
      collector
    end
  end
end

# Class constructor blocks chained with `.new` are still valid visibility scopes.
class ConstructorChain
  def build
    obj = Class.new do
      private

      def private_property
      end

      protected

      def protected_property
      end
    end.new
  end
end

# Module constructor blocks passed as call arguments accept the same spacing.
class Wrapper
  include(Module.new do
    private

    def helper
    end
  end)
end

# Brace-block module constructors passed as call arguments also accept blank lines.
Runner.singleton_class.prepend Module.new {
  private

    def list_tests
    end
}

# Constant assignment wrappers do not propagate macro scope into root
# receiverful blocks, so `private` inside this DSL block is not checked.
module Authentication
  Authenticate = CommandClass.new(
    inputs: %i[a]
  ) do
    def call
    end

    private

    def authenticator
    end
  end
end
