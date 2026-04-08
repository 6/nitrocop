def name
  client.name.upcase
end

def name(arg)
  client.name
end

def name
  compute_something
end

delegate :name, to: :client

# Class method receivers can't use delegate
def no_replies_scope
  Status.without_replies
end

def find_user
  User.find_by_email(email)
end

# Method name doesn't match delegated method — not a simple delegation
def valid?
  json.present?
end

def cdn_host
  config.asset_host
end

# Safe navigation is ignored
def author_url
  structured_data&.author_url
end

# Argument forwarding with transformation (not simple delegation)
def fetch(key)
  client.fetch(key.to_s)
end

# Argument count mismatch
def [](key, default)
  @attrs[key]
end

# Private methods are ignored — including methods after other method `end`s
# in the same private section
private

def custom_filter
  object.custom_filter
end

def logger
  Rails.logger
end

def discoverable
  Account.discoverable
end

# module_function makes methods private — delegate doesn't apply
module Helpers
  module_function

  def name
    self.name
  end
end

# module_function on a separate line before the method
module Utils
  module_function

  def label
    config.label
  end
end

# inline module_function before def
module Formatters
  module_function def format
    self.format
  end
end

# private :method_name after def — makes the method private (single symbol only)
# RuboCop's pattern (send nil? VISIBILITY_SCOPES (sym %method_name)) only matches
# when there's exactly one symbol argument
def status
  record.status
end
private :status

# EnforceForPrefixed: false patterns with non-CallNode receivers
# (these are skipped in our prefix check but tested elsewhere)

# Receiver is a method call with arguments — not a simple delegation target
def fox
  bar(42).fox
end

# Method args don't match (param is used as receiver, not forwarded)
def fox(bar)
  bar.fox
end

# Local-variable receivers are not matched by RuboCop's Rails/Delegate pattern.
def delete(account_env_var)
  account_env_var.delete(account_env_var)
end

# Prism normalizes `def !@` to method name `!`, but RuboCop compares against `!@`.
def !@
  !value
end

# module_function :name AFTER the def — RuboCop skips these
module Adapter
  def adapters
    Adapter.adapters
  end; module_function :adapters

  def register(name, condition)
    Adapter.register(name, condition)
  end; module_function :register
end

# module_function :name on a subsequent line in the same module scope
module SecurityUtils
  def secure_compare(a, b)
    OpenSSL.fixed_length_secure_compare(a, b)
  end

  module_function :secure_compare
end

# module_function in ancestor module — class nested inside module
# RuboCop's module_function_declared? checks ALL ancestors, not just immediate scope
module Open4
  module_function :open4
  class SpawnError < StandardError
    def exitstatus
      @status.exitstatus
    end
  end
end

# Endless method with chained call — not a simple delegation
class Foo
  def formatted_name = value.name.upcase
end

# Endless method with argument mismatch — not a delegation
class Bar
  def lookup(key) = data.find
end

# Private with trailing space — `private ` (with trailing space) on its own line
class Bar
  private
  def size
    @range.size
  end
end

# module_function in outer ancestor module (after nested class) suppresses delegation.
# RuboCop's module_function_declared? checks ALL ancestors, including outer modules.
# Pattern from antiwork/gumroad: module_function in StripePaymentMethodHelper (outer)
# suppresses delegation defs in nested ExtensionMethods module.
module StripeHelper
  module ExtensionMethods
    def to_customer_id
      to_customer.id
    end
  end

  class StripeUtils
    def self.build_error(msg)
      {error: msg}
    end
  end

  module_function

  def build(token:)
    card = {token:}
    card.extend(ExtensionMethods)
    card
  end
end

# module_function in outer module after nested class << self suppresses delegation.
# Pattern from palkan/anyway_config: class Trace inside module Tracing, with
# module_function declared in Tracing after the class << self singleton block.
module Tracing
  class Trace
    def clear() = value.clear
  end

  class << self
    def capture
      yield
    end

    private

    def source_stack
      []
    end
  end

  module_function

  def trace!(type, **opts)
    yield
  end
end

# private block with INDENTED def — private at lower indent than def
# Pattern from rails/rails: `private\n    def mkdir(dirs)`
class TestHelper
  private
    def mkdir(dirs)
      FileUtils.mkdir(dirs)
    end

    def entries
      @changelog.entries
    end
end

# `private def` inline modifier — private on same line as def
# Pattern from ruby/debug, codetriage/CodeTriage, pakyow/pakyow, ruby/rbs
class Config
  private def config
    self.class.config
  end

  private def parse_config_value(name, valstr)
    self.class.parse_config_value(name, valstr)
  end
end

# private def with question mark method — codetriage/CodeTriage pattern
class Issue
  private def pr_attached_with_issue?(pull_request_hash)
    self.class.pr_attached_with_issue?(pull_request_hash)
  end
end

# private def inside nested module — pakyow/pakyow pattern
module Support
  module Hookable
    module ClassMethods
      private def known_event?(event)
        self.class.known_event?(event)
      end

      private def hook_pipeline
        self.class.hook_pipeline
      end
    end
  end
end

# private def inside nested class — ruby/rbs pattern
module Collection
  module Sources
    class Stdlib
      private def lookup(name, version)
        REPO.lookup(name, version)
      end
    end
  end
end

# private block with indented def delegating to constant — antiwork/gumroad pattern
class ChargeProcessor
  private
    def paypal_api
      PaypalChargeProcessor.paypal_api
    end
end

# private inside a block body under a constant assignment should suppress
# nested delegations in that same block body.
Factory = Class.new do
  private

  def value
    @source.value
  end
end

# private inside a class_exec block should suppress nested delegations there too.
class Container
  def self.build
    class_exec do
      private

      def resource_class
        self.class.resource_class
      end
    end
  end
end

# private inside a class nested under an if-body should still suppress the def.
if tracker_class
  class ERBTracker < tracker_class
    private

    def source
      template.source
    end
  end
end

# Heredoc text containing `do not` should not look like a Ruby `do` block opener
# and cancel the enclosing private section for later methods.
class InteractiveIgnorer
  private

  def show_help
    say <<-HELP
q - Quit, do not update ignored warnings
    HELP
  end

  def warning_action(action, warning)
    case action
    when "i"
      ignore warning
    when "u"
      unignore warning
    end
  end

  def ignore(warning)
    @ignore_config.ignore warning
  end

  def unignore(warning)
    @ignore_config.unignore warning
  end
end
