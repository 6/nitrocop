# Public method
def foo
  puts 'bar'
end

def initialize
  @x = 1
end

# Another documented method
def bar
  42
end

# Private methods don't need docs (default RequireForNonPublicMethods: false)
private

def private_method
  42
end

protected

def protected_method
  42
end

# Inline private
private def inline_private
  42
end

# Documentation for modular method
module_function def modular_method
  42
end

# Documentation for keywords method
ruby2_keywords def keyword_method
  42
end

# private_class_method is non-public, skipped by default
private_class_method def self.secret
  42
end

# TODO: fix this
# Real documentation follows the annotation
def annotated_then_doc
  42
end

# Private with indented def (common Ruby style)
class IndentedPrivate
  private
    def indented_private_method
      42
    end

  protected
    def indented_protected_method
      42
    end
end

# Private inside class << self followed by private section
module ActionCable
    class Base
      class << self
      end
      private
        def delegate_connection_identifiers
          42
        end
    end
end

# Private in nested class with different indentation
class Container
  class Nested
    private
      def deeply_nested_private
        42
      end
  end
end

# Retroactive private :method_name makes method non-public (no docs needed)
class RetroactivePrivate
  def secret_method
    42
  end
  private :secret_method
end

# Retroactive protected :method_name makes method non-public
class RetroactiveProtected
  def guarded_method
    42
  end
  protected :guarded_method
end

# public re-establishes visibility after private section
class PublicAfterPrivate
  private

  def secret
    42
  end

  public

  # Documented public method after public keyword
  def visible
    42
  end
end

# Nested class between private and def should not reset visibility
class NestedClassAfterPrivate
  private

  class Inner
    # Documented inner method
    def inner_method
      42
    end
  end

  def still_private_method
    42
  end
end

# Nested module between private and def should not reset visibility
class NestedModuleAfterPrivate
  private

  module Helper
  end

  def also_private_method
    42
  end
end

# Private with trailing whitespace on the private line
class TrailingWhitespacePrivate
  private

  def trailing_ws_method
    42
  end
end

# private(def ...) makes the method private
private(def paren_private_method
  42
end)

# protected(def ...) makes the method protected
protected(def paren_protected_method
  42
end)

# Single-line class defs should not break peer scope tracking
class Webfinger
  class Error < StandardError; end
  class GoneError < Error; end
  class RedirectError < Error; end

  # Documented public method
  def perform
    42
  end

  private

  def secret_helper
    42
  end

  def another_helper
    42
  end
end

# Migration-style: single-line class + private section
class BackfillMigration
  class Account < ActiveRecord::Base; end
  class User < ActiveRecord::Base; end
  class Status < ActiveRecord::Base; end

  # Documented up method
  def up
    process_logs
  end

  private

  def process_logs
    42
  end

  def process_users
    42
  end
end

# Comment with <<WORD before private should not break visibility tracking
class CommentWithHeredocSyntax
  # This comment mentions <<EOF heredoc syntax
  private

  def method_after_comment_with_heredoc
    42
  end
end

# Trailing comment with <<WORD should not trigger heredoc tracking
class TrailingCommentHeredoc
  x = 1 # use <<HEREDOC for multiline
  private

  def method_after_trailing_comment
    42
  end
end

# Line starting with # that has <<WORD should not trigger heredoc
class CommentLineHeredoc
  # Heredocs use <<~RUBY or <<-SQL syntax
  private

  def method_after_comment_line
    42
  end
end

# rubocop:disable directive between doc comment and def should not suppress docs.
# RuboCop sees the doc comment above the blank line via ast_with_comments.

# Create a meaningful operation name from the semantic convention
# @see https://opentelemetry.io/docs/specs/semconv/general/trace/

# rubocop:disable Metrics/CyclomaticComplexity,Metrics/PerceivedComplexity
def method_with_rubocop_disable_after_doc
  42
end

# Converts a value from database input to the appropriate ruby type.
#
# @param value [String] value to deserialize
#
# @return [Object] deserialized value

# rubocop:disable Style/RescueModifier
def method_with_rubocop_disable_after_yard_doc
  42
end

# It accepts an array of coordinates
# and an optional radius

# rubocop:disable Metrics/MethodLength
def method_with_rubocop_disable_after_short_doc
  42
end

# Inline prefix containing `private` makes method non-public (no docs needed)
memoized internal private def memoized_private_method
  42
end

# protected in a mixed inline prefix also makes method non-public
some_decorator protected def decorated_protected_method
  42
end

# Sentence-style "Note" comments are documentation, unlike "Note:" annotations.
# Note to recompute the bitmaps on a resize
def note_to_method
  42
end

# Note that the compatibility hacks only apply to the old ctors
def note_that_method
  42
end

# private inside the same conditional branch still makes the method non-public
if condition
  private
  def same_branch_private
    42
  end
end

# Multi-byte UTF-8 in comments must not panic the annotation keyword scanner
# All ΔE* formulae were designed for JND thresholds
def method_with_unicode_comment
  42
end

# [c, d] ← to ← [to.begin, to.end]
def method_with_arrow_comment
  42
end

# Postfix `unless` on class should not leak wrapped_comment_depth into class body
class HashExtensions
  # Returns a new hash with transformed keys
  def transform_keys
    42
  end

  # Destructively transforms keys
  def transform_keys!
    42
  end
end unless defined? ActiveSupport

# Postfix `if` on class should not leak wrapped_comment_depth into class body
class ConditionalClass
  # Documented method inside conditional class
  def conditional_documented
    42
  end
end if true

# Methods nested inside a call-wrapped def body should check docs independently
module PWN
  private_class_method def self.pry
    Pry.class_eval do
      # This is documentation for evaluate_ruby
      def evaluate_ruby(code)
        42
      end
    end
  end
end

# ruby2_keywords in a private section: method is non-public, no docs needed
class PrivateRuby2Keywords
  private

  ruby2_keywords def method_missing(method, *args, &block)
    42
  end
end

# module_function in a private section: method is non-public per RuboCop
class PrivateModuleFunction
  private

  module_function def internal_helper
    42
  end
end

# Methods inside private def wrapper body are NOT themselves private
module Jb
  module CollectionRendererExtension
    private def render_collection(template)
      obj = super
      # Documented singleton method
      def obj.body; []; end
    end
  end
end

# Comments inside a Class.new block passed as a call argument belong to the method
stub_const(
  "ActiveJob::QueueAdapters::AppsignalTestAdapter",
  Class.new(ActiveJob::QueueAdapters::InlineAdapter) do
    # Adapter used in our test suite to add provider data to the job
    # data, as is done by Rails provided ActiveJob adapters.
    #
    # This implementation is based on the inline adapter.
    def enqueue(job)
      ActiveJob::Base.execute(job.serialize)
    end
  end
)

## TODO clean this up
def double_hash_todo_method
  42
end

### TODO clean this up
def triple_hash_todo_method
  42
end

# An inline comment on the enclosing branch opener counts for the first method in the branch.
if condition
else # older versions
  def producer
    42
  end
end

# An inline comment on the enclosing begin line also counts for the first method in the body.
begin # relative path
  def files_relative_path
    42
  end
end

begin
ensure # cleanup
  def ensured_method
    42
  end
end
