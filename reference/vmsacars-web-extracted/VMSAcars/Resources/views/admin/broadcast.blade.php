{{ Form::model(null, ['route' => ['vmsacars.admin.broadcast'], 'method' => 'post']) }}
<div class="card border-blue-bottom">
  <div class="content">
    <div class="header">
      <h3>Broadcast</h3>
      <p class="description">
        Send a broadcast message to your ACARS pilots
      </p>
    </div>
  </div>
  <div class="content">
    {{ Form::input('text', 'message', '', ['class' => 'form-control']) }}
    <div class="text-right">
      {{ Form::button('Send', ['type' => 'submit', 'class' => 'btn btn-success']) }}
    </div>
  </div>
</div>
{{ Form::close() }}
