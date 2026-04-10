obj.then { |x| x.do_something }
1.then { |x| x + 1 }
foo.then(&method(:bar))
fulfilled_future(1).then(2, &-> v { v })
obj.map { |x| x }
obj.each { |x| x }
obj.select { |x| x }
