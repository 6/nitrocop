# coding: US-ASCII
class Crashy
  def re(line)
    /\A(\w+), \[([^#]*) #(\d+)\]\s+(\w+) -- (\w*): ([\x0-\xff]*)/ =~ line
  end

  def test_reraise_write_errors
    logger = Logger.new(c, :reraise_write_errors=>[e])
  end

  def test_linear_performance
    pre = ->(n) {[Regexp.new("a?" * n + "a" * n), "a" * n]}
    assert_linear_performance([10, 29], pre: pre) do |re, s|
      re =~ s
    end
  end
end
