# nitrocop-config: EnforcedStyle: omit_parentheses

# ambiguity in the receiver chain keeps parentheses
def ignored_organisations_string
  (ignored_organisations || []).join(", ")
end

def convert_grouped_input(value)
  %w[1 2 3].map { |key| value[key] }.join("-")
end

# inner calls inside blocks keep parentheses when the block is directly wrapped
# by another call
self.result = run_callbacks(:execute) do
  execute
end

# inner calls used as super arguments keep parentheses
super(errors.local_attribute(attribute))

# unary descendant arguments keep the outer call parentheses
def input_field
  some_helper(form_field_name, value, id: form_field_id, readonly: !editable?)
end

# lambda descendants make the outer call ambiguous enough to keep parens
options = {
  tag: :div,
  tags_formatter: ->(tags) { tags.join(" ") }
}.merge(options)

# Lambda body whose parent expression is a setter/assignment-like call: the
# lambda block's parent is a call, so RuboCop's call_in_argument_with_block?
# allows parentheses for the inner call.
options[:converter] = ->(x) { ObjectThing.converter(x) }

# Negative-numeric range descendants are ambiguous in the same way as bare
# negative numbers, so the outer call's parens stay.
def slice_domain
  s[1..-1].join(".")
end

# Negative numeric inside an exclusive range descendant
def trim_extension
  Concurrent::Array.new(nested_property_names[0...-1])
end

# Ternary inside a string interpolation descendant of an argument is still
# ambiguous content; the outer call keeps parentheses.
def ordered_languages
  Language.published.order("name #{options[:reverse] ? "DESC" : "ASC"}")
end

# Rescue clauses make the main body expression non-value-returning, so
# shorthand keyword arguments keep their parentheses.
module Clusters
  class InstallBuildCloudJob < ApplicationJob
    def perform(build_cloud, user)
      Clusters::InstallBuildCloud.execute(build_cloud:, user:)
    rescue StandardError
      nil
    end
  end
end

# Ternary `else` branch carrying an assignment whose RHS is a parenthesized
# call. RuboCop's `assignment_in_condition?` allows the parens because the
# surrounding ternary is `conditional?`. The grandparent in nitrocop's
# parent_stack is `TernaryBranch`, which must satisfy the same allowance.
def self._subst(rec, opts, tag, mode)
  case mode.downcase
  when "exist"
    ref.nil? ? value = false : value = ref.is_tagged_with?(tag, :ns => "*")
  when "registry"
    ref.nil? ? value = "" : value = registry_data(ref, tag, ohash)
  end
end

# Hash-value-omission inside a single-line block keeps parens via
# `node.parent&.single_line?`. The Block / CallLikeBlockBody parent must
# carry the block's source span so `parent_is_single_line()` returns true
# for `let(:x) { create(:y, foo:) }`-style spec helpers and inline
# `.map { |x| Foo.new(x:) }` chains.
RSpec.describe Favorites::ForUser do
  let!(:cluster) { create(:cluster, account:) }
  let!(:project) { create(:project, cluster:, account:) }
  let(:component) { Component.new(name:, label:, input_type:, params:) }
end

class Theme
  def self.default_themes
    DEFAULT_THEMES.map { |name, icon| new(name:, icon:) }
  end
end

def points_creator
  payload = data
            .compact
            .map { |location| location.merge(user_id:) }
end

# Hash literal reachable through a non-send/non-hash descendant of an arg.
# RuboCop's `hash_literal_in_arguments?` runs `node.descendants.any?` over the
# whole subtree once a direct arg is a send call, so a `{}` nested inside an
# `||=` op-asgn arg or an `lvasgn = {...}` arg keeps the parens too.
def allowed_ous_tree_each
  ous = {}
  hous = {}
  ous.each { |ou| create_ou_tree(ou, hous[dc_path] ||= {}, ou[0].split(',')) }
end

def workflow_setup
  workflow = described_class.new(values = {:running_pre_dialog => false}, admin)
end

# Single-statement `else` body of a `case` expression. RuboCop's
# `assignment_in_condition?` exempts because the lvasgn's grandparent is the
# `case` node (`conditional?`). nitrocop must push `ConditionalBody` for the
# else branch's single statement so that grandparent matches.
def format_timezone_else(new_time)
  case ftype
  when "tl"
    new_time = I18n.l(new_time)
  else
    new_time = I18n.l(new_time)
  end
end
