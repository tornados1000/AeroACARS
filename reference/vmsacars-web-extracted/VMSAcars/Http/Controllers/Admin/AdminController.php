<?php

namespace Modules\VMSAcars\Http\Controllers\Admin;

use App\Contracts\Controller;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;
use Laracasts\Flash\Flash;
use Modules\VMSAcars\Models\Config;
use Modules\VMSAcars\Models\Rule;
use Modules\VMSAcars\Services\AcarsService;

class AdminController extends Controller
{
    public function __construct(
        private readonly AcarsService $acarsSvc
    ) {}

    public function index()
    {
        $config = Config::select('*')->orderBy('order', 'asc')->get();
        $rules = Rule::select('*')->orderBy('order', 'asc')->get();

        return view('vmsacars::admin.index', [
            'all_config' => $config,
            'all_rules'  => $rules,
        ]);
    }

    public function config(Request $request)
    {
        // Update the config, and check the license at the end
        $config = Config::select(['id', 'value'])->get();

        foreach ($config as $c) {
            $value = $request->input($c->id);
            $c->value = trim($value);
            $c->save();
        }

        // Validate license, redirect with error if its invalid
        if (!empty($request->input('license_key'))) {
            $check = $this->acarsSvc->validateLicense($request->input('license_key'));
            if ($check['error'] === true) {
                Flash::error('Error validating license: '.$check['message']);
            } else {
                Flash::success('License validated!');
            }
        }

        return redirect(route('vmsacars.admin.index'));
    }

    public function rules(Request $request)
    {
        // Update the rules
        $rules = Rule::all();
        foreach ($rules as $rule) {
            if ($rule->has_parameter) {
                $rule->parameter = $request[$rule->id.'_parameter'];
            }

            $rule->points = $request[$rule->id.'_points'];
            $rule->delay = $request[$rule->id.'_delay'];
            $rule->cooldown = $request[$rule->id.'_cooldown'];
            $rule->repeatable = get_truth_state($request[$rule->id.'_repeatable']);
            $rule->enabled = get_truth_state($request[$rule->id.'_enabled']);
            $rule->save();
        }

        return redirect(route('vmsacars.admin.index'));
    }

    /**
     * Send a broadcast message to ACARS clients
     *
     *
     * @return mixed
     */
    public function broadcast(Request $request)
    {
        if (empty($request->input('message'))) {
            Flash::error('Message is empty!');

            return redirect(route('vmsacars.admin.index'));
        }

        $user = Auth::user();
        $check = $this->acarsSvc->sendBroadcast($user, $request->input('message'));
        if ($check['error'] === true) {
            Flash::error('Error sending broadcast: '.$check['message']);
        } else {
            Flash::success('Broadcast sent!');
        }

        return redirect(route('vmsacars.admin.index'));
    }
}
