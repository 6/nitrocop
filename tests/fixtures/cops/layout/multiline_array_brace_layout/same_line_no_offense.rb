# nitrocop-config: EnforcedStyle: same_line

files = [
  ["db_adapters/mysql_adapter.rb", <<-EOS],
    module DbAdapters::MysqlAdapter
    end
  EOS
]

changes = [
  ContentChange.new(range: nil, text: <<RBS)
class Hello
end
RBS
]
