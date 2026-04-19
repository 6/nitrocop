# nitrocop-config: EnforcedStyle: assign_inside_condition

# No else branch — not flagged
x = if foo
      1
    end

# case without else — not flagged
x = case foo
    when "a"
      1
    when "b"
      2
    end

# Already assigns inside condition (the "good" pattern)
if foo
  x = 1
else
  x = 2
end

# Multi-line branches with SingleLineConditionsOnly=true (default) — not flagged
x = if foo
      something
      1
    else
      2
    end

x = if foo
      1
    else
      something_else
      2
    end

# Method call that looks like ternary but is not assignment
bar << foo? ? 1 : 2

# Simple non-conditional assignment
x = 1
@y = 2
$z = "hello"

# FP fix: ternary with parenthesized branches (begin_type? in RuboCop)
x = cond ? (a) : b
x = cond ? a : (b)
x = cond ? (a) : (b)
success = (foo ? (bar == '0') : (baz == '1'))

# FP fix: if/else with parenthesized branch expression
x = if foo
      (bar ? 'a' : 'b')
    else
      'c'
    end

# FP fix: assignment nested inside another assignment (part_of_ignored_node?)
y = [1].map { |v| x = v > 0 ? 'pos' : 'neg'; x }
trunc = lambda { |s| s = s.length > 10 ? s : s[0..10]; s }

# FP fix: assignment inside ||= begin...end
@cache ||= begin
  path = windows ? c_chef_dir : other_dir
  clean(path)
end

# FP fix: RuboCop's autocorrect crashes when an elsif/else keyword is
# less indented than the assignment LHS. RuboCop drops the offense in
# that case (Shopify/krane pattern).
def refute_resource_exists(type, name)
  client = if %w(daemonset deployment replicaset statefulset).include?(type)
    apps_v1_kubeclient
 elsif %w(ingress networkpolicy).include?(type)
   networking_v1_kubeclient
  else
    kubeclient
  end
end

# FP fix: RuboCop's autocorrect crashes on an empty elsif body (calls
# `tail(nil)`). Offense is dropped (brianmario/mysql2 pattern).
dll_path = if ENV['RUBY_MYSQL2_LIBMYSQL_DLL']
  ENV['RUBY_MYSQL2_LIBMYSQL_DLL']
elsif File.exist?('vendor/libmysql.dll')
  'vendor/libmysql.dll'
elsif defined?(RubyInstaller)
  # RubyInstaller-2.4+ native build doesn't need DLL preloading
else
  'libmysql.dll'
end

# FP fix: chained assignment where the inner assignment's column is
# greater than the `else`/`end` column so `column - node_column` goes
# negative in RuboCop's autocorrect (feedbin pattern).
def show
  if saved_search.present?
    per_page = params[:per_page] = if params[:include_entries] == "true"
      100
    else
      limit
    end
  end
end
