get :index, { user_id: 1 }, { "ACCEPT" => "text/html" }
            ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `get`.

post :create, { name: "foo" }, { "X-TOKEN" => "abc" }
              ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `post`.

put :update, { id: 1 }, { "Authorization" => "Bearer xyz" }
             ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `put`.

get :show, :id => 12
           ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `get`.

get :edit, :id => 12
           ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `get`.

get :new, :thing => { :name => "Herbert the thing" }
          ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `get`.

post :create, confirmation_data
              ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `post`.

post :create, id: 7, format: :rss
              ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `post`.

put :update, :id => 12
             ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `put`.

put :update, :id => 12, :thing => {:name => "Jorje"}
             ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `put`.

delete :destroy, :id => 12
                 ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `delete`.

get :show, id: "product", format: "pdf"
           ^ Rails/HttpPositionalArguments: Use keyword arguments instead of positional arguments for http call: `get`.
