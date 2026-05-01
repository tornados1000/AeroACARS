<?php

namespace Modules\VMSAcars\Http\Controllers\Api;

use App\Contracts\Controller;
use App\Http\Resources\Flight as FlightResource;
use App\Models\PirepField;
use App\Models\User;
use App\Repositories\Criteria\WhereCriteria;
use App\Repositories\FlightRepository;
use App\Services\VersionService;
use Composer\Semver\Comparator;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;
use Modules\VMSAcars\Http\Errors\VmsAcarsUpgradeRequired;
use Modules\VMSAcars\Http\Resources\ConfigCollection;
use Modules\VMSAcars\Http\Resources\PirepField as PirepFieldResource;
use Modules\VMSAcars\Http\Resources\Rule as RuleResource;
use Modules\VMSAcars\Models\Config;
use Modules\VMSAcars\Models\Rule;
use Modules\VMSAcars\Services\AcarsService;
use Prettus\Repository\Criteria\RequestCriteria;
use Prettus\Repository\Exceptions\RepositoryException;

class ApiController extends Controller
{
    public function __construct(
        private readonly AcarsService $acarsSvc,
        private readonly FlightRepository $flightRepo,
        private readonly VersionService $versionSvc
    ) {}

    /**
     * @return \Illuminate\Http\JsonResponse
     */
    public function config(Request $request)
    {
        // Check the minimum version of vmsACARS that can connect
        $user_agent = $request->header('user-agent');
        if (str_contains($user_agent, 'vmsacars')) {
            $version = explode(' ', $user_agent)[1];

            if (Comparator::greaterThan($version, '2.0.0')) {
                $minimum_version = config('vmsacars.minimum_acars_version');
                if (Comparator::lessThan($version, $minimum_version)) {
                    throw new VmsAcarsUpgradeRequired();
                }
            }
        }

        $config = Config::select(['id', 'type', 'value'])->orderBy('order', 'asc')->get();
        $res = (new ConfigCollection($config))->toArray($request);

        /** @var User $user */
        $user = Auth::user();
        $res['app_name'] = config('app.name');
        $res['acars_user_id'] = $this->acarsSvc->getUserId($user);
        $res['acars_airline_id'] = $this->acarsSvc->getAirlineId();
        $res['module_version'] = config('vmsacars.version');
        $res['phpvms_version'] = $this->versionSvc->getCurrentVersion();

        $fields = PirepField::all();

        $res['fields'] = PirepFieldResource::collection($fields);
        $res['plugins'] = [];
        $res['rules'] = $this->rules($request);
        $res['settings'] = [

        ];

        $res['units'] = [
            'd' => setting('units.distance'),
            'f' => setting('units.fuel'),
            's' => setting('units.speed'),
            't' => setting('units.temperature'),
            'v' => setting('units.volume'),
            'w' => setting('units.weight'),
        ];

        // Read from the KVP table

        return response()->json($res);
    }

    /**
     * @return mixed
     */
    public function search(Request $request)
    {
        /** @var \App\Models\User $user */
        $user = Auth::user();

        $where = [
            'active'  => true,
            'visible' => true,
        ];

        // Allow the option to bypass some of these restrictions for the searches
        if (!$request->filled('ignore_restrictions') || $request->get('ignore_restrictions') === '0') {
            if (setting('pilots.restrict_to_company')) {
                $where['airline_id'] = $user->airline_id;
            }

            if (setting('pilots.only_flights_from_current')) {
                $where['dpt_airport_id'] = $user->curr_airport_id;
            }
        }

        try {
            $this->flightRepo->resetCriteria();
            $this->flightRepo->searchCriteria($request);
            $this->flightRepo->pushCriteria(new WhereCriteria($request, $where, [
                'airline' => ['active' => true],
            ]));

            $this->flightRepo->pushCriteria(new RequestCriteria($request));

            $with = [
                'airline',
                'field_values',
                'simbrief' => function ($query) use ($user) {
                    return $query->with('aircraft')->where('user_id', $user->id);
                },
            ];

            $relations = [];
            $flights = $this->flightRepo->with($with)->paginate();
        } catch (RepositoryException $e) {
            return response($e, 503);
        }

        // TODO: Remove any flights here that a user doesn't have permissions to
        foreach ($flights as $flight) {
            if (in_array('subfleets', $relations)) {
                $this->flightSvc->filterSubfleets($user, $flight);
            }
        }

        return FlightResource::collection($flights);
    }

    /**
     * Return all of the rules
     *
     *
     * @return \Illuminate\Http\Resources\Json\AnonymousResourceCollection
     */
    public function rules(Request $request)
    {
        $rules = Rule::where(['enabled' => true])->get();

        return RuleResource::collection($rules);
    }

    public function plugins(Request $request) {}
}
