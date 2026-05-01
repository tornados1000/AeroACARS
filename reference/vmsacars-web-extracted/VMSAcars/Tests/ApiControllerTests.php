<?php

namespace Modules\VMSAcars\Tests;

use App\Models\User;
use App\Services\ModuleService;
use Illuminate\Support\Facades\Artisan;
use Illuminate\Support\Facades\Config;
use Modules\VMSAcars\Http\Errors\VmsAcarsUpgradeRequired;
use Modules\VMSAcars\Providers\VMSAcarsServiceProvider;
use Tests\CreatesApplication;
use Tests\TestCase;

final class ApiControllerTests extends TestCase {
    use CreatesApplication;

    protected function setUp(): void
    {
        parent::setUp();

        /** @var ModuleService $moduleSvc */
        $moduleSvc = app(ModuleService::class);
        $moduleSvc->addModule('vmsacars');
        Artisan::call('migrate', ['--env' => 'testing', '--force' => true]);

        $this->app->register(VMSAcarsServiceProvider::class, true);

        Config::set('vmsacars.minimum_acars_version', '2.0.526');
    }

    public function test_acars_dont_allow_below_2_0_526_version() {

        /** @var User $user */
        $user = User::factory()->create();
        // $routes = Route::getRoutes();
        // $route = $routes->getByName("vmsacars.api.config");

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 2.0.451'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(400);
        $body = $response->json();

        $upgrade_message = new VmsAcarsUpgradeRequired();
        $this->assertEquals($upgrade_message->getErrorDetails(), $body['details']);
    }

    public function test_acars_allow_below_2_0_526_version() {

        /** @var User $user */
        $user = User::factory()->create();

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 2.0.451'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(400);
        $body = $response->json();

        $upgrade_message = new VmsAcarsUpgradeRequired();
        $this->assertEquals($upgrade_message->getErrorDetails(), $body['details']);
    }

    public function test_acars_allow_same() {

        /** @var User $user */
        $user = User::factory()->create();

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 2.0.526'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(200);
    }

    public function test_acars_allow_greater() {

        /** @var User $user */
        $user = User::factory()->create();
        // $routes = Route::getRoutes();
        // $route = $routes->getByName("vmsacars.api.config");

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 2.0.527'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(200);
    }

    public function test_acars_allow_1x() {

        /** @var User $user */
        $user = User::factory()->create();
        // $routes = Route::getRoutes();
        // $route = $routes->getByName("vmsacars.api.config");

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 1.0.1.1112'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(200);
    }

    public function test_acars_allow_dev() {

        /** @var User $user */
        $user = User::factory()->create();
        // $routes = Route::getRoutes();
        // $route = $routes->getByName("vmsacars.api.config");

        $uri = '/api/vmsacars/config';
        $headers = [
            'x-api-key'  => $user->api_key,
            'user-agent' => 'vmsacars 2.1.11-dev32'
        ];

        $response = $this->getJson($uri, $headers);
        $response->assertStatus(200);
    }
}
