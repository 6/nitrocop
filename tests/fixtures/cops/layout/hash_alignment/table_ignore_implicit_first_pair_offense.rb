# nitrocop-config: EnforcedStyle: table, table, ignore_implicit
def format_route(rails_route, args)
  { :path => format_path(rails_route),
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    :verb => format_verb(rails_route),
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    :desc => args[:desc],
    ^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    :options => args[:options] }
end

hash = {
  "aaa" =>
  ^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    1,
  "bb"  => 2
}
