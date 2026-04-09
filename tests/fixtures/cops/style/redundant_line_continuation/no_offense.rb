=begin
bundle lock \
  --add-platform aarch64-linux \
  --add-platform x86_64-linux
=end

=begin rdoc
  ssh key used to login to remote instances\
=end

foo \
  "string"

foo \
  (bar)

super \
  bar

super \
  (bar)

x = 'hello' \
  'world'

message = \
  "hello" +
  "world"

sources = \
  foo |
  bar

y = 1
z = 2

x = "line with a literal backslash \\"
y = "another \\ line"

1 \
  + 2

bar \
  if foo

bar \
  unless foo

obj
 .foo(42) \

 .bar

output = Whenever.cron \
<<-file
  every "weekday" do
    command "blah"
  end
file

change(Commentaire, :count).by(0).and \
  change(ContactForm, :count).by(1)

contain_exactly(a, b).or \
  contain_exactly(c, d)

foo \
  %w[bar]

1 \
  % 2

=begin
x = 'hello' \
  'world'
=end

helper_method \
  def ordergroups_for_adding
  Ordergroup.undeleted.order(:name)
end

def foo
  yield \
    (bar)
end

existing_autoload_paths = \
  deps.autoload_paths.select { |dir| File.directory?(dir) } -
  deps.autoload_once_paths

expect(view.include_gon(need_tag: false)).to eq( \
  'window.gon={};' +
  'gon.int=1;'
)

gsub_file "config/rubber/rubber-percona.yml", /^db_backup_cmd.*$/, \
  "backup command" +
  " updated"
