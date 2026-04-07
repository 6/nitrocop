get :index, { user_id: 1 }, { "ACCEPT" => "text/html" }
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/HttpPositionalArguments: Use keyword arguments for HTTP request methods.

post :create, { name: "foo" }, { "X-TOKEN" => "abc" }
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/HttpPositionalArguments: Use keyword arguments for HTTP request methods.

put :update, { id: 1 }, { "Authorization" => "Bearer xyz" }
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/HttpPositionalArguments: Use keyword arguments for HTTP request methods.
