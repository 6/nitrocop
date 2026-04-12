#!/usr/bin/env ruby
def reload!
^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

def foo
^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  puts 'bar'
end

def method; end
^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

def another_method
^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# TODO: fix this later
def annotated_method
^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# rubocop:disable Style/Foo
def directive_method
^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# frozen_string_literal: true
def interpreter_directive_method
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

module_function def undocumented_modular
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# Documentation above the line is for the wrapping call, not the def
memoize def memoized_method
        ^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# Outputs an element tag.
register_element def custom_tag(**attrs, &content) = nil
                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

module Postmark
  module HashHelper
    # Compatibility shim
    def enhance_with_compatibility_warning(hash)
      def hash.[](key)
      ^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
        42
      end
    end
  end
end

class UpdateChecker
  # Returns the update check service.
  def update_check_service
    Struct.new(:origin) do
      def latest_version
      ^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
        42
      end
    end
  end
end

module Datadog
  module LibdatadogExtconfHelpers
    # Note: This helper is currently only used in the `libdatadog_api/extconf.rb`
    def self.load_libdatadog_or_get_issue
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
      42
    end
  end
end

class Sender
  private

  if CLOSEABLE_QUEUES
    def send_loop
    ^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
      42
    end
  else
    def send_loop
    ^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
      42
    end
  end
end

class StatSerializer
  private

  if RUBY_VERSION < '3'
    def metric_name_to_string(metric_name)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
      metric_name.to_s
    end
  else
    def metric_name_to_string(metric_name)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
      metric_name.to_s
    end
  end
end

if FEATURE_AVAILABLE
  def conditional_method
  ^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end
end

# Documentation above the line is for the wrapping modifier, not the def
def rdoc_dummy_method; super; end if false
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

class MultiRetroactiveProtected
  def helper_one
  ^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end

  def helper_two
  ^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end
  protected :helper_one, :helper_two
end

class RetroactivePrivateString
  def string_method
  ^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end
  private "string_method"
end

protected (def spaced_paren_protected
           ^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end)

def json!
^ Style/DocumentationMethod: Missing method documentation comment.

def articles_courses_scope
^ Style/DocumentationMethod: Missing method documentation comment.

def scope
^ Style/DocumentationMethod: Missing method documentation comment.

# A doc comment with inline rubocop:disable is treated as a directive, not documentation # rubocop:disable Layout/LineLength
def method_with_inline_rubocop_disable
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# Undocumented methods inside postfix-modifier class body are still offenses
class PostfixUnlessClass
  def undocumented_in_unless_class
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end
end unless some_condition

# Methods inside private def body that lack docs should still be flagged
module Wrapper
  private def enclosing_private_method
    def obj.undocumented_singleton; end
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  end
end

def json!
^ Style/DocumentationMethod: Missing method documentation comment.

album.instance_eval { def name=; raise; end }
                      ^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

__skip__ = def new_field(**kwargs)
           ^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

__skip__ = def new_input_field(**kwargs)
           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

# Comment above belongs to assignment, not the inner def
__skip__ = def documented_but_stolen(**kwargs)
           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
  42
end

# Comment above belongs to the singleton class, not the inner def
class << some_obj; def singleton_stolen; end; end
                   ^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.

def json!
^ Style/DocumentationMethod: Missing method documentation comment.

def reload!
^ Style/DocumentationMethod: Missing method documentation comment.

def self.supports_ranges?
^ Style/DocumentationMethod: Missing method documentation comment.

def __id__
^ Style/DocumentationMethod: Missing method documentation comment.

def self.compile(str, options)
^ Style/DocumentationMethod: Missing method documentation comment.

def helper.append_javascript_pack_tag(name, **options)
^ Style/DocumentationMethod: Missing method documentation comment.

def cocoapods_generate_specs_cp_repos_dir
^ Style/DocumentationMethod: Missing method documentation comment.

class RetroactivePrivateInstanceOnly
  def self.supports_ranges?
  ^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end

  def supports_ranges?; self.class.supports_ranges?; end
  private :supports_ranges?
end

module OpalInterpolation
  %x{
    if (typeof Opal.eval === 'undefined') {
      #{
        def self.eval(str)
        ^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
          42
        end
      }
    }
  }

  %x{
    (function() {
      'use strict';
      #{
        def __id__
        ^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
          42
        end
      }
    })()
  }
end

class BeginEnsureWrapper
  # Documented outer method
  def outer
    begin
      # Comment belongs to the begin/ensure wrapper, not the nested def.
      def helper.append_javascript_pack_tag(name, **options)
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
        42
      end
    ensure
      cleanup
    end
  end
end

module RescueWrappedWithJson
  # Get parsed JSON from the output, raising an error if parsing fails
  #
  #: () -> Hash[Symbol, untyped]
  def json!
  ^^^^^^^^^ Style/DocumentationMethod: Missing method documentation comment.
    42
  end
rescue
  []
end

def json!
^ Style/DocumentationMethod: Missing method documentation comment.

def reload!
^ Style/DocumentationMethod: Missing method documentation comment.

def self.supports_ranges?
^ Style/DocumentationMethod: Missing method documentation comment.

def __id__
^ Style/DocumentationMethod: Missing method documentation comment.

def self.compile(str, options)
^ Style/DocumentationMethod: Missing method documentation comment.

def helper.append_javascript_pack_tag(name, **options)
^ Style/DocumentationMethod: Missing method documentation comment.

def cocoapods_generate_specs_cp_repos_dir
^ Style/DocumentationMethod: Missing method documentation comment.
